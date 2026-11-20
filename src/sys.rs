//! Accès bas niveau à Windows : liste des processus en un seul appel noyau,
//! chemins / lignes de commande, fenêtres, ports, actions (tuer, suspendre, priorité),
//! élévation, mémoire en veille, plan d'alimentation, remplacement du Gestionnaire des tâches.
#![allow(clippy::upper_case_acronyms, non_snake_case, non_camel_case_types)]

use std::collections::HashMap;
use std::ffi::c_void;
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HWND, LPARAM, LUID};
use windows_sys::Win32::System::Threading as th;
use windows_sys::Win32::UI::WindowsAndMessaging as wm;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(p: &[u16]) -> String {
    let n = p.iter().position(|&c| c == 0).unwrap_or(p.len());
    String::from_utf16_lossy(&p[..n])
}

// ---------- ntdll (API non documentées mais stables depuis Windows XP, utilisées par System Informer) ----------
#[repr(C)]
#[derive(Clone, Copy)]
struct UNICODE_STRING {
    Length: u16,
    MaximumLength: u16,
    Buffer: *const u16,
}

#[repr(C)]
struct SYSTEM_PROCESS_INFORMATION {
    NextEntryOffset: u32,
    NumberOfThreads: u32,
    WorkingSetPrivateSize: i64,
    HardFaultCount: u32,
    NumberOfThreadsHighWatermark: u32,
    CycleTime: u64,
    CreateTime: i64,
    UserTime: i64,
    KernelTime: i64,
    ImageName: UNICODE_STRING,
    BasePriority: i32,
    UniqueProcessId: usize,
    InheritedFromUniqueProcessId: usize,
    HandleCount: u32,
    SessionId: u32,
    UniqueProcessKey: usize,
    PeakVirtualSize: usize,
    VirtualSize: usize,
    PageFaultCount: u32,
    PeakWorkingSetSize: usize,
    WorkingSetSize: usize,
    QuotaPeakPagedPoolUsage: usize,
    QuotaPagedPoolUsage: usize,
    QuotaPeakNonPagedPoolUsage: usize,
    QuotaNonPagedPoolUsage: usize,
    PagefileUsage: usize,
    PeakPagefileUsage: usize,
    PrivatePageCount: usize,
    ReadOperationCount: i64,
    WriteOperationCount: i64,
    OtherOperationCount: i64,
    ReadTransferCount: i64,
    WriteTransferCount: i64,
    OtherTransferCount: i64,
}

#[repr(C)]
struct SYSTEM_THREAD_INFORMATION {
    KernelTime: i64,
    UserTime: i64,
    CreateTime: i64,
    WaitTime: u32,
    StartAddress: *const c_void,
    UniqueProcess: usize,
    UniqueThread: usize,
    Priority: i32,
    BasePriority: i32,
    ContextSwitches: u32,
    ThreadState: u32,
    WaitReason: u32,
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformation(class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn NtSetSystemInformation(class: u32, buf: *const c_void, len: u32) -> i32;
    fn NtQueryInformationProcess(h: HANDLE, class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn NtSuspendProcess(h: HANDLE) -> i32;
    fn NtResumeProcess(h: HANDLE) -> i32;
}

const SYSTEM_PROCESS_INFORMATION_CLASS: u32 = 5;
const SYSTEM_MEMORY_LIST_INFORMATION: u32 = 80;
const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004u32 as i32;

/// Un processus tel que le noyau le décrit, à un instant donné.
#[derive(Clone)]
pub struct Raw {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub create_time: i64,
    /// temps CPU cumulé (unités de 100 ns)
    pub cpu_time: i64,
    /// « Mémoire » du Gestionnaire des tâches : plage de travail privée
    pub private_ws: u64,
    /// mémoire engagée (privée)
    pub commit: u64,
    pub ws: u64,
    pub io_read: u64,
    pub io_write: u64,
    pub io_other: u64,
    pub threads: u32,
    pub handles: u32,
    pub session: u32,
    pub base_prio: i32,
    pub suspended: bool,
}

/// Une seule lecture noyau pour tous les processus (comme System Informer), tampon réutilisé.
pub fn query_processes(buf: &mut Vec<u8>) -> Vec<Raw> {
    if buf.len() < 512 * 1024 {
        buf.resize(512 * 1024, 0);
    }
    loop {
        let mut ret = 0u32;
        let st = unsafe { NtQuerySystemInformation(SYSTEM_PROCESS_INFORMATION_CLASS, buf.as_mut_ptr().cast(), buf.len() as u32, &mut ret) };
        if st == STATUS_INFO_LENGTH_MISMATCH {
            let n = (ret as usize).max(buf.len()) + 128 * 1024;
            buf.resize(n, 0);
            continue;
        }
        if st < 0 {
            return Vec::new();
        }
        break;
    }
    let mut out = Vec::with_capacity(400);
    let mut off = 0usize;
    loop {
        // SAFETY : le noyau a rempli le tampon avec une liste chaînée d'entrées alignées.
        let p = unsafe { &*(buf.as_ptr().add(off) as *const SYSTEM_PROCESS_INFORMATION) };
        let name = if p.ImageName.Buffer.is_null() {
            if p.UniqueProcessId == 0 { "Processus inactif du système".to_string() } else { "System".to_string() }
        } else {
            let s = unsafe { std::slice::from_raw_parts(p.ImageName.Buffer, p.ImageName.Length as usize / 2) };
            String::from_utf16_lossy(s)
        };
        // Suspendu = tous les threads en attente pour la raison « Suspended » (5)
        let nt = p.NumberOfThreads as usize;
        let threads_ptr = unsafe { (p as *const SYSTEM_PROCESS_INFORMATION).add(1) as *const SYSTEM_THREAD_INFORMATION };
        let mut suspended = nt > 0 && p.UniqueProcessId != 0;
        for i in 0..nt {
            let t = unsafe { &*threads_ptr.add(i) };
            if !(t.ThreadState == 5 && t.WaitReason == 5) {
                suspended = false;
                break;
            }
        }
        out.push(Raw {
            pid: p.UniqueProcessId as u32,
            ppid: p.InheritedFromUniqueProcessId as u32,
            name,
            create_time: p.CreateTime,
            cpu_time: p.UserTime + p.KernelTime,
            private_ws: p.WorkingSetPrivateSize.max(0) as u64,
            commit: p.PrivatePageCount as u64,
            ws: p.WorkingSetSize as u64,
            io_read: p.ReadTransferCount.max(0) as u64,
            io_write: p.WriteTransferCount.max(0) as u64,
            io_other: p.OtherTransferCount.max(0) as u64,
            threads: p.NumberOfThreads,
            handles: p.HandleCount,
            session: p.SessionId,
            base_prio: p.BasePriority,
            suspended,
        });
        if p.NextEntryOffset == 0 {
            break;
        }
        off += p.NextEntryOffset as usize;
        if off >= buf.len() {
            break;
        }
    }
    out
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn open(pid: u32, access: u32) -> Option<Handle> {
    let h = unsafe { th::OpenProcess(access, 0, pid) };
    if h.is_null() { None } else { Some(Handle(h)) }
}

/// Date de création (100 ns, comme `CreateTime` du noyau) d'un processus ouvert.
fn creation_time(h: &Handle) -> Option<i64> {
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    (unsafe { th::GetProcessTimes(h.0, &mut c, &mut e, &mut k, &mut u) } != 0).then(|| ft(c) as i64)
}

/// Ouvre le processus `pid` **seulement s'il s'agit toujours du même** (même date de création que
/// `ctime`) : un PID libéré peut être réutilisé par un autre processus entre la mesure et l'action.
fn open_checked(pid: u32, ctime: i64, access: u32, what: &str) -> Result<Handle, String> {
    let h = open(pid, access | th::PROCESS_QUERY_LIMITED_INFORMATION).ok_or_else(|| access_err(what))?;
    match creation_time(&h) {
        Some(t) if t == ctime => Ok(h),
        Some(_) => Err("ce processus s’est terminé (son PID a été réutilisé)".into()),
        None => Err(last_err()),
    }
}

/// Chemin complet de l'exe (accès « limité » : marche sans admin pour les processus de l'utilisateur).
pub fn process_path(pid: u32) -> Option<String> {
    let h = open(pid, th::PROCESS_QUERY_LIMITED_INFORMATION)?;
    let mut buf = [0u16; 1024];
    let mut n = buf.len() as u32;
    let ok = unsafe { th::QueryFullProcessImageNameW(h.0, 0, buf.as_mut_ptr(), &mut n) };
    (ok != 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
}

/// Ligne de commande (Windows 8.1+, accès limité suffisant).
pub fn process_cmdline(pid: u32) -> Option<String> {
    let h = open(pid, th::PROCESS_QUERY_LIMITED_INFORMATION)?;
    let mut buf: Vec<u8> = vec![0; 4096];
    loop {
        let mut ret = 0u32;
        let st = unsafe { NtQueryInformationProcess(h.0, PROCESS_COMMAND_LINE_INFORMATION, buf.as_mut_ptr().cast(), buf.len() as u32, &mut ret) };
        if st == STATUS_INFO_LENGTH_MISMATCH || st == 0x8000_0005u32 as i32 || st == 0xC000_0023u32 as i32 {
            if ret as usize <= buf.len() || ret > 1 << 20 {
                return None;
            }
            buf.resize(ret as usize, 0);
            continue;
        }
        if st < 0 {
            return None;
        }
        break;
    }
    let us = unsafe { &*(buf.as_ptr() as *const UNICODE_STRING) };
    if us.Buffer.is_null() || us.Length == 0 {
        return None;
    }
    let s = unsafe { std::slice::from_raw_parts(us.Buffer, us.Length as usize / 2) };
    Some(String::from_utf16_lossy(s))
}

// ---------- Fenêtres ----------
/// pid → titre de sa première fenêtre visible (non masquée par DWM).
pub fn windows() -> HashMap<u32, String> {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
        let map = unsafe { &mut *(lp as *mut HashMap<u32, String>) };
        unsafe {
            if wm::IsWindowVisible(h) == 0 || !wm::GetWindow(h, wm::GW_OWNER).is_null() {
                return 1;
            }
            let mut cloaked = 0u32;
            windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute(h, 14 /* DWMWA_CLOAKED */, (&mut cloaked as *mut u32).cast(), 4);
            // DWM_CLOAKED_SHELL (2) = sur un autre bureau virtuel : c'est une vraie fenêtre.
            // Seules les fenêtres masquées par l'app elle-même ou héritées (1, 4) sont ignorées.
            if cloaked & !2 != 0 {
                return 1;
            }
            let len = wm::GetWindowTextLengthW(h);
            if len == 0 {
                return 1;
            }
            let mut pid = 0u32;
            wm::GetWindowThreadProcessId(h, &mut pid);
            if let std::collections::hash_map::Entry::Vacant(e) = map.entry(pid) {
                let mut buf = vec![0u16; len as usize + 1];
                let n = wm::GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
                e.insert(String::from_utf16_lossy(&buf[..n.max(0) as usize]));
            }
        }
        1
    }
    let mut map: HashMap<u32, String> = HashMap::new();
    unsafe { wm::EnumWindows(Some(cb), &mut map as *mut _ as LPARAM) };
    map
}

/// Toutes les fenêtres de premier niveau d'un processus (pour une fermeture propre).
fn top_windows_of(pid: u32) -> Vec<HWND> {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
        let (pid, v) = unsafe { &mut *(lp as *mut (u32, Vec<HWND>)) };
        let mut p = 0u32;
        unsafe { wm::GetWindowThreadProcessId(h, &mut p) };
        if p == *pid && unsafe { wm::GetWindow(h, wm::GW_OWNER).is_null() } {
            v.push(h);
        }
        1
    }
    let mut st: (u32, Vec<HWND>) = (pid, Vec::new());
    unsafe { wm::EnumWindows(Some(cb), &mut st as *mut _ as LPARAM) };
    st.1
}

pub fn focus_window_of(pid: u32) -> bool {
    let ws = top_windows_of(pid);
    let Some(&h) = ws.iter().find(|&&h| unsafe { wm::IsWindowVisible(h) } != 0) else { return false };
    unsafe {
        if wm::IsIconic(h) != 0 {
            wm::ShowWindow(h, wm::SW_RESTORE);
        }
        wm::SetForegroundWindow(h) != 0
    }
}

// ---------- Ports ----------
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub tcp: bool,
    pub port: u16,
    /// écoute seulement sur la boucle locale (127.0.0.1 / ::1)
    pub local_only: bool,
}

/// Ports TCP en écoute et UDP ouverts, par processus. Deux appels IP Helper par famille, ~0,2 ms.
pub fn ports() -> HashMap<u32, Vec<Port>> {
    use windows_sys::Win32::NetworkManagement::IpHelper as ip;
    let mut map: HashMap<u32, Vec<Port>> = HashMap::new();
    let fetch = |udp: bool, af: u32| -> Vec<u8> {
        let mut size = 0u32;
        let mut buf: Vec<u8> = Vec::new();
        for _ in 0..4 {
            let r = unsafe {
                if udp {
                    ip::GetExtendedUdpTable(buf.as_mut_ptr().cast(), &mut size, 0, af, ip::UDP_TABLE_OWNER_PID, 0)
                } else {
                    ip::GetExtendedTcpTable(buf.as_mut_ptr().cast(), &mut size, 0, af, ip::TCP_TABLE_OWNER_PID_LISTENER, 0)
                }
            };
            if r == 0 {
                return buf;
            }
            if r != 122 {
                return Vec::new();
            }
            buf.resize(size as usize + 1024, 0);
            size = buf.len() as u32;
        }
        Vec::new()
    };
    fn rows(buf: &[u8], row: usize) -> Vec<&[u8]> {
        if buf.len() < 4 {
            return Vec::new();
        }
        let n = u32::from_ne_bytes(buf[0..4].try_into().unwrap()) as usize;
        (0..n).filter_map(|i| buf.get(4 + i * row..4 + (i + 1) * row)).collect()
    }
    let u = |b: &[u8], o: usize| u32::from_ne_bytes(b[o..o + 4].try_into().unwrap());
    let port = |v: u32| u16::from_be((v & 0xFFFF) as u16);
    let mut add = |pid: u32, p: Port| {
        let v = map.entry(pid).or_default();
        if !v.iter().any(|x| x.tcp == p.tcp && x.port == p.port) {
            v.push(p);
        }
    };
    // TCP v4 : state, local, lport, remote, rport, pid
    for r in rows(&fetch(false, 2), 24) {
        add(u(r, 20), Port { tcp: true, port: port(u(r, 8)), local_only: r[4] == 127 });
    }
    // TCP v6 : addr[16], scope, lport, raddr[16], rscope, rport, state, pid
    for r in rows(&fetch(false, 23), 56) {
        let lo = r[..15].iter().all(|&b| b == 0) && r[15] == 1;
        add(u(r, 52), Port { tcp: true, port: port(u(r, 20)), local_only: lo });
    }
    for r in rows(&fetch(true, 2), 12) {
        add(u(r, 8), Port { tcp: false, port: port(u(r, 4)), local_only: r[0] == 127 });
    }
    for r in rows(&fetch(true, 23), 28) {
        let lo = r[..15].iter().all(|&b| b == 0) && r[15] == 1;
        add(u(r, 24), Port { tcp: false, port: port(u(r, 20)), local_only: lo });
    }
    for v in map.values_mut() {
        v.sort_by_key(|p| (!p.tcp, p.port));
    }
    map
}

// ---------- Totaux système ----------
fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

/// (inactif, noyau+utilisateur) cumulés, en 100 ns.
pub fn system_times() -> (u64, u64) {
    let (mut i, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe { th::GetSystemTimes(&mut i, &mut k, &mut u) };
    (ft(i), ft(k) + ft(u))
}

pub struct Mem {
    pub total: u64,
    pub avail: u64,
    pub commit_total: u64,
    pub commit_limit: u64,
}

pub fn memory() -> Mem {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    unsafe { GlobalMemoryStatusEx(&mut m) };
    Mem {
        total: m.ullTotalPhys,
        avail: m.ullAvailPhys,
        commit_total: m.ullTotalPageFile - m.ullAvailPageFile,
        commit_limit: m.ullTotalPageFile,
    }
}

pub fn cpu_count() -> u32 {
    unsafe { th::GetActiveProcessorCount(0xFFFF) }.max(1)
}

/// Mémoire en veille (cache) en octets — lisible seulement en admin.
pub fn standby_bytes() -> Option<u64> {
    #[repr(C)]
    struct MemList {
        zero: usize,
        free: usize,
        modified: usize,
        modified_no_write: usize,
        bad: usize,
        standby: [usize; 8],
        repurposed: [usize; 8],
        modified_pagefile: usize,
    }
    let mut m: MemList = unsafe { std::mem::zeroed() };
    let st = unsafe {
        NtQuerySystemInformation(SYSTEM_MEMORY_LIST_INFORMATION, (&mut m as *mut MemList).cast(), std::mem::size_of::<MemList>() as u32, std::ptr::null_mut())
    };
    (st >= 0).then(|| m.standby.iter().sum::<usize>() as u64 * 4096)
}

// ---------- Actions ----------
pub fn kill(pid: u32, ctime: i64) -> Result<(), String> {
    let h = open_checked(pid, ctime, th::PROCESS_TERMINATE, "tuer")?;
    if unsafe { th::TerminateProcess(h.0, 1) } == 0 {
        return Err(last_err());
    }
    Ok(())
}

pub fn suspend(pid: u32, ctime: i64, on: bool) -> Result<(), String> {
    let h = open_checked(pid, ctime, 0x0800 /* PROCESS_SUSPEND_RESUME */, "suspendre")?;
    let st = unsafe { if on { NtSuspendProcess(h.0) } else { NtResumeProcess(h.0) } };
    if st < 0 { Err(format!("échec (0x{st:08X})")) } else { Ok(()) }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Prio {
    Idle,
    BelowNormal,
    Normal,
    AboveNormal,
    High,
    Realtime,
}

impl Prio {
    pub const ALL: [Prio; 6] = [Prio::Realtime, Prio::High, Prio::AboveNormal, Prio::Normal, Prio::BelowNormal, Prio::Idle];
    pub fn label(self) -> &'static str {
        match self {
            Prio::Idle => "Faible",
            Prio::BelowNormal => "Inférieure à la normale",
            Prio::Normal => "Normale",
            Prio::AboveNormal => "Supérieure à la normale",
            Prio::High => "Haute",
            Prio::Realtime => "Temps réel",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            Prio::Idle => "prio. faible",
            Prio::BelowNormal => "prio. basse",
            Prio::Normal => "",
            Prio::AboveNormal => "prio. +",
            Prio::High => "prio. haute",
            Prio::Realtime => "temps réel",
        }
    }
    /// Classe déduite de la priorité de base renvoyée par le noyau.
    pub fn from_base(b: i32) -> Prio {
        match b {
            ..=4 => Prio::Idle,
            5..=7 => Prio::BelowNormal,
            8..=9 => Prio::Normal,
            10..=12 => Prio::AboveNormal,
            13..=23 => Prio::High,
            _ => Prio::Realtime,
        }
    }
    fn class(self) -> u32 {
        match self {
            Prio::Idle => th::IDLE_PRIORITY_CLASS,
            Prio::BelowNormal => th::BELOW_NORMAL_PRIORITY_CLASS,
            Prio::Normal => th::NORMAL_PRIORITY_CLASS,
            Prio::AboveNormal => th::ABOVE_NORMAL_PRIORITY_CLASS,
            Prio::High => th::HIGH_PRIORITY_CLASS,
            Prio::Realtime => th::REALTIME_PRIORITY_CLASS,
        }
    }
}

pub fn set_priority(pid: u32, ctime: i64, p: Prio) -> Result<(), String> {
    let h = open_checked(pid, ctime, th::PROCESS_SET_INFORMATION, "modifier")?;
    if unsafe { th::SetPriorityClass(h.0, p.class()) } == 0 {
        return Err(last_err());
    }
    Ok(())
}

/// Demande poliment aux fenêtres de se fermer (comme cliquer sur ×). Renvoie false s'il n'y en a aucune.
pub fn close_gracefully(pid: u32) -> bool {
    let ws = top_windows_of(pid);
    for &h in &ws {
        unsafe { wm::PostMessageW(h, wm::WM_CLOSE, 0, 0) };
    }
    !ws.is_empty()
}

fn last_err() -> String {
    std::io::Error::last_os_error().to_string()
}

fn access_err(what: &str) -> String {
    if is_elevated() {
        format!("accès refusé (impossible de {what} ce processus protégé)")
    } else {
        format!("accès refusé — relance Vigie en admin pour {what} ce processus")
    }
}

/// Vide la plage de travail d'un processus sans le fermer : ses pages partent en mémoire « modifiée / en veille »
/// et la RAM redevient disponible ; il les relira (un peu plus lentement) quand il en aura besoin.
/// Renvoie les octets libérés (plage de travail avant − après).
pub fn trim(pid: u32, ctime: i64) -> Result<u64, String> {
    use windows_sys::Win32::System::ProcessStatus::{K32EmptyWorkingSet, K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    let h = open_checked(pid, ctime, 0x0100 /* PROCESS_SET_QUOTA */, "mettre en veille")?;
    let ws = || {
        let mut c: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if unsafe { K32GetProcessMemoryInfo(h.0, &mut c, c.cb) } != 0 { c.WorkingSetSize as u64 } else { 0 }
    };
    let before = ws();
    if unsafe { K32EmptyWorkingSet(h.0) } == 0 {
        return Err(last_err());
    }
    Ok(before.saturating_sub(ws()))
}

/// Précharge la mémoire privée d'un processus (pages renvoyées au fichier d'échange) : Windows relit
/// en arrière-plan, en grosses lectures, au lieu de défauts de page un par un au premier clic.
/// Les pages reviennent en cache « en veille » et sont remappées sans disque quand l'app y touche.
/// Renvoie les octets demandés.
pub fn prefetch(pid: u32, ctime: i64) -> Result<u64, String> {
    use windows_sys::Win32::System::Memory::*;
    let h = open_checked(pid, ctime, th::PROCESS_QUERY_INFORMATION | 0x0008 /* PROCESS_VM_OPERATION */ | 0x0010 /* PROCESS_VM_READ */, "réveiller")?;
    let mut ranges: Vec<WIN32_MEMORY_RANGE_ENTRY> = Vec::new();
    let mut addr: usize = 0;
    let mut total: u64 = 0;
    loop {
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let n = unsafe { VirtualQueryEx(h.0, addr as *const _, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) };
        if n == 0 {
            break;
        }
        let usable = mbi.State == MEM_COMMIT && mbi.Type == MEM_PRIVATE && mbi.Protect & (PAGE_GUARD | PAGE_NOACCESS) == 0 && mbi.Protect != 0;
        if usable {
            ranges.push(WIN32_MEMORY_RANGE_ENTRY { VirtualAddress: mbi.BaseAddress, NumberOfBytes: mbi.RegionSize });
            total += mbi.RegionSize as u64;
        }
        let next = (mbi.BaseAddress as usize).saturating_add(mbi.RegionSize);
        if next <= addr || total > 8 << 30 {
            break;
        }
        addr = next;
    }
    if ranges.is_empty() {
        return Ok(0);
    }
    if unsafe { PrefetchVirtualMemory(h.0, ranges.len(), ranges.as_ptr(), 0) } == 0 {
        return Err(last_err());
    }
    Ok(total)
}

/// Processus suspendus **maintenant** (lecture noyau fraîche, ~3 ms) : la dernière mesure peut
/// dater d'une seconde, et suspendre deux fois empile les suspensions.
pub fn suspended_now() -> std::collections::HashSet<(u32, i64)> {
    query_processes(&mut Vec::new()).into_iter().filter(|r| r.suspended).map(|r| (r.pid, r.create_time)).collect()
}

/// PID de la fenêtre au premier plan.
pub fn foreground_pid() -> u32 {
    let mut pid = 0u32;
    unsafe { wm::GetWindowThreadProcessId(wm::GetForegroundWindow(), &mut pid) };
    pid
}

// ---------- Élévation et privilèges ----------
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    static CACHE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| unsafe {
        let mut tok: HANDLE = std::ptr::null_mut();
        if th::OpenProcessToken(th::GetCurrentProcess(), TOKEN_QUERY, &mut tok) == 0 {
            return false;
        }
        let tok = Handle(tok);
        let mut e = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut n = 0u32;
        let ok = GetTokenInformation(tok.0, TokenElevation, (&mut e as *mut TOKEN_ELEVATION).cast(), 4, &mut n);
        ok != 0 && e.TokenIsElevated != 0
    })
}

pub fn enable_privilege(name: &str) -> bool {
    use windows_sys::Win32::Security::*;
    unsafe {
        let mut tok: HANDLE = std::ptr::null_mut();
        if th::OpenProcessToken(th::GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut tok) == 0 {
            return false;
        }
        let tok = Handle(tok);
        let mut luid = LUID::default();
        if LookupPrivilegeValueW(std::ptr::null(), wide(name).as_ptr(), &mut luid) == 0 {
            return false;
        }
        let tp = TOKEN_PRIVILEGES { PrivilegeCount: 1, Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }] };
        AdjustTokenPrivileges(tok.0, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut()) != 0
            && windows_sys::Win32::Foundation::GetLastError() == 0
    }
}

/// Relance l'exe avec l'invite UAC. Renvoie true si l'utilisateur a accepté.
pub fn run_elevated(args: &str, wait: bool) -> bool {
    use windows_sys::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe = wide(&exe.to_string_lossy());
    let verb = wide("runas");
    let params = wide(args);
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = exe.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = wm::SW_SHOWNORMAL;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return false;
    }
    if !info.hProcess.is_null() {
        let h = Handle(info.hProcess);
        if wait {
            unsafe { th::WaitForSingleObject(h.0, 15_000) };
        }
    }
    true
}

/// Vide la liste « en veille » (cache de fichiers) : la RAM redevient libre immédiatement. Admin requis.
pub fn purge_standby() -> Result<(), String> {
    if !is_elevated() {
        return Err("nécessite les droits admin".into());
    }
    enable_privilege("SeProfileSingleProcessPrivilege");
    let cmd: u32 = 4; // MemoryPurgeStandbyList
    let st = unsafe { NtSetSystemInformation(SYSTEM_MEMORY_LIST_INFORMATION, (&cmd as *const u32).cast(), 4) };
    if st < 0 { Err(format!("refusé par Windows (0x{st:08X})")) } else { Ok(()) }
}

// ---------- Plan d'alimentation ----------
pub const HIGH_PERF: &str = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";

fn guid_str(g: &windows_sys::core::GUID) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        g.data1, g.data2, g.data3, g.data4[0], g.data4[1], g.data4[2], g.data4[3], g.data4[4], g.data4[5], g.data4[6], g.data4[7]
    )
}

fn parse_guid(s: &str) -> Option<windows_sys::core::GUID> {
    let h: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if h.len() != 32 {
        return None;
    }
    let v = u128::from_str_radix(&h, 16).ok()?;
    Some(windows_sys::core::GUID::from_u128(v))
}

pub fn active_power_scheme() -> Option<String> {
    use windows_sys::Win32::System::Power::PowerGetActiveScheme;
    let mut p: *mut windows_sys::core::GUID = std::ptr::null_mut();
    if unsafe { PowerGetActiveScheme(std::ptr::null_mut(), &mut p) } != 0 || p.is_null() {
        return None;
    }
    let s = guid_str(unsafe { &*p });
    unsafe { windows_sys::Win32::Foundation::LocalFree(p.cast()) };
    Some(s)
}

pub fn set_power_scheme(guid: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Power::PowerSetActiveScheme;
    let g = parse_guid(guid).ok_or("GUID invalide")?;
    let r = unsafe { PowerSetActiveScheme(std::ptr::null_mut(), &g) };
    if r == 0 { Ok(()) } else { Err(std::io::Error::from_raw_os_error(r as i32).to_string()) }
}

/// Nom d'un plan tel que Windows l'affiche (« Ultimate Performance », « Utilisation normale »…),
/// lu par `PowerReadFriendlyName` : un appel d'API, aucun processus lancé.
pub fn power_label(guid: &str) -> Option<String> {
    use windows_sys::Win32::System::Power::PowerReadFriendlyName;
    let g = parse_guid(guid)?;
    let mut buf = [0u16; 256];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe { PowerReadFriendlyName(std::ptr::null_mut(), &g, std::ptr::null(), std::ptr::null(), buf.as_mut_ptr().cast(), &mut size) };
    (r == 0).then(|| from_wide(&buf)).filter(|s| !s.is_empty())
}

/// Plan actif : (GUID, nom, déjà « performances maximales » ?). Un seul passage, sans powercfg.
pub fn active_power_info() -> Option<(String, String, bool)> {
    let g = active_power_scheme()?;
    let label = power_label(&g).unwrap_or_else(|| g.clone());
    let l = label.to_lowercase();
    let max = g == HIGH_PERF || g == "e9a42b02-d5df-448d-aa00-03f14749eb61" || ["ultimate", "optimal", "high perf", "élevées", "elevees", "maximal"].iter().any(|k| l.contains(k));
    Some((g, label, max))
}

pub fn power_scheme_name(guid: &str) -> String {
    if let Some(l) = power_label(guid) {
        return l;
    }
    match guid {
        HIGH_PERF => "Performances élevées".into(),
        "381b4222-f694-41f0-9685-ff5bb260df2e" => "Utilisation normale".into(),
        "a1841308-3541-4fab-bc81-f71556f20b4a" => "Économie d’énergie".into(),
        "e9a42b02-d5df-448d-aa00-03f14749eb61" => "Performances optimales".into(),
        g => format!("plan {}", &g[..8.min(g.len())]),
    }
}

// ---------- Remplacer le Gestionnaire des tâches (Ctrl+Maj+Échap) ----------
const IFEO: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\taskmgr.exe";

/// Valeur « Debugger » actuelle de taskmgr.exe (même technique que Process Explorer).
pub fn taskmgr_debugger() -> Option<String> {
    use windows_sys::Win32::System::Registry::*;
    let mut buf = [0u16; 1024];
    let mut n = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, wide(IFEO).as_ptr(), wide("Debugger").as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut n)
    };
    (r == 0).then(|| from_wide(&buf))
}

/// Copie inscrite dans IFEO : `%ProgramFiles%\Vigie\Vigie.exe`. Un exe sous `C:\Dev\…\dist`
/// est modifiable sans droits admin : l'inscrire comme « débogueur » de taskmgr.exe ouvrirait une
/// porte (n'importe quel programme pourrait le remplacer et être lancé à la place du Gestionnaire).
pub fn taskmgr_target() -> Option<std::path::PathBuf> {
    std::env::var_os("ProgramFiles").map(|p| std::path::PathBuf::from(p).join("Vigie").join("Vigie.exe"))
}

pub fn taskmgr_replaced() -> bool {
    let Some(d) = taskmgr_debugger() else { return false };
    let d = d.trim_matches('"').to_string();
    let target = taskmgr_target().map(|t| t.to_string_lossy().into_owned()).unwrap_or_default();
    let me = std::env::current_exe().map(|m| m.to_string_lossy().into_owned()).unwrap_or_default();
    d.eq_ignore_ascii_case(&target) || d.eq_ignore_ascii_case(&me)
}

/// Écrit/efface la clé (admin requis : appelé par une instance relancée avec `--set-taskmgr on|off`).
pub fn set_taskmgr_direct(on: bool) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::*;
    let r = unsafe {
        if on {
            // copie dans Program Files (dossier protégé), et c'est ce chemin-là qui est inscrit
            let me = std::env::current_exe().map_err(|e| e.to_string())?;
            let target = taskmgr_target().ok_or("ProgramFiles introuvable")?;
            std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| format!("Program Files : {e}"))?;
            if !me.eq(&target) {
                std::fs::copy(&me, &target).map_err(|e| format!("copie vers {} : {e}", target.display()))?;
            }
            let v = wide(&format!("\"{}\"", target.display()));
            RegSetKeyValueW(HKEY_LOCAL_MACHINE, wide(IFEO).as_ptr(), wide("Debugger").as_ptr(), REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32)
        } else {
            let r = RegDeleteKeyValueW(HKEY_LOCAL_MACHINE, wide(IFEO).as_ptr(), wide("Debugger").as_ptr());
            if let Some(t) = taskmgr_target() {
                let _ = std::fs::remove_file(&t);
                let _ = std::fs::remove_dir(t.parent().unwrap());
            }
            if r == 2 { 0 } else { r }
        }
    };
    if r == 0 { Ok(()) } else { Err(std::io::Error::from_raw_os_error(r as i32).to_string()) }
}

pub fn set_taskmgr(on: bool) -> Result<(), String> {
    if is_elevated() {
        return set_taskmgr_direct(on);
    }
    if !run_elevated(if on { "--set-taskmgr on" } else { "--set-taskmgr off" }, true) {
        return Err("autorisation administrateur refusée".into());
    }
    // Vérification vue de l'instance non élevée (et éventuellement virtualisée) : la clé HKLM et
    // la copie dans Program Files doivent être réelles, pas redirigées.
    let copie = taskmgr_target().map(|t| t.is_file()).unwrap_or(false);
    match (taskmgr_replaced() == on, !on || copie) {
        (true, true) => Ok(()),
        (false, _) => Err("la modification n’a pas été appliquée".into()),
        (true, false) => Err("clé écrite mais la copie dans Program Files est introuvable".into()),
    }
}

// ---------- Icônes des exe ----------
/// Icône de l'exe en RGBA (taille `px`), via le shell. Appelé hors du thread d'interface.
pub fn exe_icon(path: &str, large: bool) -> Option<(usize, usize, Vec<u8>)> {
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SMALLICON, SHGetFileInfoW};
    let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
    let flags = SHGFI_ICON | if large { SHGFI_LARGEICON } else { SHGFI_SMALLICON };
    let r = unsafe { SHGetFileInfoW(wide(path).as_ptr(), 0, &mut info, std::mem::size_of::<SHFILEINFOW>() as u32, flags) };
    if r == 0 || info.hIcon.is_null() {
        return None;
    }
    let hicon = info.hIcon;
    let out = unsafe {
        let mut ii: wm::ICONINFO = std::mem::zeroed();
        if wm::GetIconInfo(hicon, &mut ii) == 0 {
            wm::DestroyIcon(hicon);
            return None;
        }
        let mut bm: BITMAP = std::mem::zeroed();
        GetObjectW(ii.hbmColor as _, std::mem::size_of::<BITMAP>() as i32, (&mut bm as *mut BITMAP).cast());
        let (w, h) = (bm.bmWidth.max(0) as usize, bm.bmHeight.max(0) as usize);
        let mut res = None;
        if w > 0 && h > 0 && !ii.hbmColor.is_null() {
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w as i32;
            bmi.bmiHeader.biHeight = -(h as i32);
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            let mut px = vec![0u8; w * h * 4];
            let dc = GetDC(std::ptr::null_mut());
            let n = GetDIBits(dc, ii.hbmColor, 0, h as u32, px.as_mut_ptr().cast(), &mut bmi, DIB_RGB_COLORS);
            // masque (icônes anciennes sans alpha)
            let has_alpha = px.chunks_exact(4).any(|c| c[3] != 0);
            if !has_alpha && !ii.hbmMask.is_null() {
                let mut mask = vec![0u8; w * h * 4];
                GetDIBits(dc, ii.hbmMask, 0, h as u32, mask.as_mut_ptr().cast(), &mut bmi, DIB_RGB_COLORS);
                for (c, m) in px.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                    c[3] = if m[0] == 0 { 255 } else { 0 };
                }
            }
            ReleaseDC(std::ptr::null_mut(), dc);
            if n > 0 {
                for c in px.chunks_exact_mut(4) {
                    c.swap(0, 2); // BGRA → RGBA
                }
                res = Some((w, h, px));
            }
        }
        if !ii.hbmColor.is_null() {
            DeleteObject(ii.hbmColor as _);
        }
        if !ii.hbmMask.is_null() {
            DeleteObject(ii.hbmMask as _);
        }
        wm::DestroyIcon(hicon);
        res
    };
    out
}

// ---------- Ouvrir / divers ----------
/// L'explorateur de l'utilisateur (C:\Dev\explorateur) s'il est là, sinon celui de Windows.
pub fn find_explorateur() -> Option<std::path::PathBuf> {
    let r = std::path::Path::new(r"C:\Dev\explorateur");
    [r.join("dist").join("Explorateur.exe"), r.join("target").join("release").join("explorateur.exe")].into_iter().find(|p| p.is_file())
}

pub fn open_location(path: &str) {
    let p = std::path::Path::new(path);
    let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    if let Some(ex) = find_explorateur() {
        if std::process::Command::new(ex).arg(&dir).spawn().is_ok() {
            return;
        }
    }
    let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{path}")).spawn();
}

/// Lance une commande sans fenêtre console et attend sa fin.
pub fn run_hidden(exe: &str, args: &[&str]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let o = std::process::Command::new(exe).args(args).creation_flags(0x0800_0000).output().map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

/// Un jeu ou une app tourne en plein écran (D3D exclusif, ou mode « occupé » / présentation) :
/// Vigie se comporte alors comme une fenêtre réduite (mesures espacées, aucune image).
pub fn fullscreen_app() -> bool {
    use windows_sys::Win32::UI::Shell::{QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};
    let mut st = 0;
    (unsafe { SHQueryUserNotificationState(&mut st) } == 0) && matches!(st, QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)
}

/// `%APPDATA%` (Roaming) sans dépendance supplémentaire.
pub fn appdata() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(std::path::PathBuf::from)
}

pub fn current_session() -> u32 {
    let mut s = 0u32;
    unsafe { windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId(th::GetCurrentProcessId(), &mut s) };
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn ports_listed() {
        let m = super::ports();
        eprintln!("{} pids, ex: {:?}", m.len(), m.iter().take(3).collect::<Vec<_>>());
        assert!(!m.is_empty());
    }
}

#[cfg(test)]
mod action_tests {
    use super::*;
    #[test]
    fn suspend_priority_kill() {
        let _verrou = crate::TEST_PROCESSUS.lock().unwrap_or_else(|e| e.into_inner());
        let mut child = std::process::Command::new("ping").args(["-n", "30", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap();
        let pid = child.id();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let ct = query_processes(&mut Vec::new()).into_iter().find(|r| r.pid == pid).unwrap().create_time;
        suspend(pid, ct, true).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let mut buf = Vec::new();
        let r = query_processes(&mut buf).into_iter().find(|r| r.pid == pid).unwrap();
        assert!(r.suspended, "devrait être suspendu");
        suspend(pid, ct, false).unwrap();
        set_priority(pid, ct, Prio::BelowNormal).unwrap();
        assert!(kill(pid, ct + 1).is_err(), "un PID réutilisé doit être refusé");
        let r = query_processes(&mut buf).into_iter().find(|r| r.pid == pid).unwrap();
        assert!(!r.suspended);
        assert_eq!(Prio::from_base(r.base_prio), Prio::BelowNormal);
        assert!(process_path(pid).unwrap().to_lowercase().ends_with("ping.exe"));
        assert!(process_cmdline(pid).unwrap().contains("127.0.0.1"));
        kill(pid, ct).unwrap();
        assert!(child.wait().is_ok());
    }
}

#[cfg(test)]
mod trim_tests {
    #[test]
    fn trim_frees_working_set() {
        let _verrou = crate::TEST_PROCESSUS.lock().unwrap_or_else(|e| e.into_inner());
        // un processus qui touche ~100 Mo puis attend
        let mut child = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "$b = New-Object byte[] 100000000; for($i=0;$i -lt $b.Length;$i+=4096){$b[$i]=1}; Start-Sleep 20"])
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_secs(4));
        let ct = super::query_processes(&mut Vec::new()).into_iter().find(|r| r.pid == child.id()).unwrap().create_time;
        let freed = super::trim(child.id(), ct).unwrap();
        eprintln!("libéré {} Mo", freed >> 20);
        let _ = super::kill(child.id(), ct);
        let _ = child.wait();
        assert!(freed > 50 << 20);
    }
}

#[cfg(test)]
mod reveil_tests {
    #[test]
    fn trim_then_prefetch() {
        let _verrou = crate::TEST_PROCESSUS.lock().unwrap_or_else(|e| e.into_inner());
        let mut child = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "$b = New-Object byte[] 100000000; for($i=0;$i -lt $b.Length;$i+=4096){$b[$i]=1}; Start-Sleep 20"])
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_secs(4));
        let ct = super::query_processes(&mut Vec::new()).into_iter().find(|r| r.pid == child.id()).unwrap().create_time;
        let freed = super::trim(child.id(), ct).unwrap();
        let asked = super::prefetch(child.id(), ct).unwrap();
        eprintln!("veille {} Mo, préchargement demandé {} Mo", freed >> 20, asked >> 20);
        let _ = super::kill(child.id(), ct);
        let _ = child.wait();
        assert!(asked > 90 << 20);
    }
}
