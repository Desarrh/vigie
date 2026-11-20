//! Compteurs de performance (PDH) : GPU par processus (comme le Gestionnaire des tâches),
//! débit disque et réseau totaux. Une seule requête PDH, collectée à chaque échantillon.
use crate::sys::wide;
use std::collections::HashMap;
use windows_sys::Win32::System::Performance::*;

pub struct Pdh {
    query: PDH_HQUERY,
    gpu_engine: Option<PDH_HCOUNTER>,
    gpu_mem: Option<PDH_HCOUNTER>,
    gpu_adapter_mem: Option<PDH_HCOUNTER>,
    disk_read: Option<PDH_HCOUNTER>,
    disk_write: Option<PDH_HCOUNTER>,
    net_rx: Option<PDH_HCOUNTER>,
    net_tx: Option<PDH_HCOUNTER>,
    buf: Vec<u8>,
}

#[derive(Default)]
pub struct Sample {
    /// pid → % GPU (moteur le plus chargé, comme le Gestionnaire)
    pub gpu: HashMap<u32, f32>,
    /// pid → mémoire GPU dédiée
    pub gpu_mem: HashMap<u32, u64>,
    pub gpu_total: f32,
    /// mémoire dédiée réellement utilisée sur les adaptateurs
    pub gpu_mem_total: u64,
    pub disk_read: f64,
    pub disk_write: f64,
    pub net_rx: f64,
    pub net_tx: f64,
}

unsafe impl Send for Pdh {}

const NOCAP: u32 = 0x8000;

impl Pdh {
    pub fn new(gpu: bool) -> Option<Pdh> {
        let mut q: PDH_HQUERY = std::ptr::null_mut();
        if unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut q) } != 0 {
            return None;
        }
        let add = |path: &str| -> Option<PDH_HCOUNTER> {
            let mut c: PDH_HCOUNTER = std::ptr::null_mut();
            (unsafe { PdhAddEnglishCounterW(q, wide(path).as_ptr(), 0, &mut c) } == 0).then_some(c)
        };
        let mut p = Pdh {
            query: q,
            gpu_engine: None,
            gpu_mem: None,
            gpu_adapter_mem: None,
            disk_read: add(r"\PhysicalDisk(_Total)\Disk Read Bytes/sec"),
            disk_write: add(r"\PhysicalDisk(_Total)\Disk Write Bytes/sec"),
            net_rx: add(r"\Network Interface(*)\Bytes Received/sec"),
            net_tx: add(r"\Network Interface(*)\Bytes Sent/sec"),
            buf: Vec::new(),
        };
        if gpu {
            p.gpu_engine = add(r"\GPU Engine(*)\Utilization Percentage");
            p.gpu_mem = add(r"\GPU Process Memory(*)\Dedicated Usage");
            p.gpu_adapter_mem = add(r"\GPU Adapter Memory(*)\Dedicated Usage");
        }
        unsafe { PdhCollectQueryData(q) };
        Some(p)
    }

    fn array(&mut self, c: PDH_HCOUNTER, fmt: u32, mut f: impl FnMut(&str, f64)) {
        let mut size = 0u32;
        let mut count = 0u32;
        let r = unsafe { PdhGetFormattedCounterArrayW(c, fmt, &mut size, &mut count, std::ptr::null_mut()) };
        if r != PDH_MORE_DATA || size == 0 {
            return;
        }
        if self.buf.len() < size as usize {
            self.buf.resize(size as usize, 0);
        }
        let items = self.buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        let r = unsafe { PdhGetFormattedCounterArrayW(c, fmt, &mut size, &mut count, items) };
        if r != 0 {
            return;
        }
        for i in 0..count as usize {
            let it = unsafe { &*items.add(i) };
            if it.FmtValue.CStatus > 1 {
                continue;
            }
            let name = unsafe {
                let mut n = 0;
                while *it.szName.add(n) != 0 {
                    n += 1;
                }
                String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, n))
            };
            let v = if fmt & PDH_FMT_LARGE != 0 { unsafe { it.FmtValue.Anonymous.largeValue as f64 } } else { unsafe { it.FmtValue.Anonymous.doubleValue } };
            f(&name, v);
        }
    }

    fn single(&self, c: Option<PDH_HCOUNTER>) -> f64 {
        let Some(c) = c else { return 0.0 };
        let mut v = PDH_FMT_COUNTERVALUE::default();
        if unsafe { PdhGetFormattedCounterValue(c, PDH_FMT_DOUBLE | NOCAP, std::ptr::null_mut(), &mut v) } != 0 || v.CStatus > 1 {
            return 0.0;
        }
        unsafe { v.Anonymous.doubleValue }
    }

    pub fn sample(&mut self) -> Sample {
        unsafe { PdhCollectQueryData(self.query) };
        let mut s = Sample { disk_read: self.single(self.disk_read), disk_write: self.single(self.disk_write), ..Default::default() };
        for (c, rx) in [(self.net_rx, true), (self.net_tx, false)] {
            let Some(c) = c else { continue };
            let mut sum = 0.0;
            self.array(c, PDH_FMT_DOUBLE | NOCAP, |name, v| {
                // interfaces virtuelles (filtres, isatap…) comptées en double : on les ignore
                let l = name.to_ascii_lowercase();
                if !(l.contains("isatap") || l.contains("loopback") || l.contains("teredo")) {
                    sum += v;
                }
            });
            if rx { s.net_rx = sum } else { s.net_tx = sum }
        }
        if let Some(c) = self.gpu_engine {
            // Instances : pid_1234_luid_0x..._phys_0_eng_3_engtype_3D
            let mut per_pid_type: HashMap<(u32, String), f64> = HashMap::new();
            let mut per_engine: HashMap<String, f64> = HashMap::new();
            self.array(c, PDH_FMT_DOUBLE | NOCAP, |name, v| {
                if v <= 0.0 {
                    return;
                }
                let Some(pid) = name.strip_prefix("pid_").and_then(|r| r.split('_').next()).and_then(|p| p.parse::<u32>().ok()) else { return };
                let engtype = name.rsplit("engtype_").next().unwrap_or("").to_string();
                let engine = name.split_once("_luid_").map(|(_, r)| r.to_string()).unwrap_or_default();
                *per_pid_type.entry((pid, engtype)).or_default() += v;
                *per_engine.entry(engine).or_default() += v;
            });
            for ((pid, _), v) in per_pid_type {
                let e = s.gpu.entry(pid).or_default();
                *e = e.max(v as f32);
            }
            s.gpu_total = per_engine.values().fold(0.0f64, |a, &b| a.max(b)).min(100.0) as f32;
        }
        if let Some(c) = self.gpu_adapter_mem {
            let mut t = 0u64;
            self.array(c, PDH_FMT_LARGE, |_, v| t += v.max(0.0) as u64);
            s.gpu_mem_total = t;
        }
        if let Some(c) = self.gpu_mem {
            self.array(c, PDH_FMT_LARGE, |name, v| {
                if let Some(pid) = name.strip_prefix("pid_").and_then(|r| r.split('_').next()).and_then(|p| p.parse::<u32>().ok()) {
                    *s.gpu_mem.entry(pid).or_default() += v.max(0.0) as u64;
                }
            });
            // Windows renvoie parfois des valeurs absurdes (allocations partagées comptées en entier) :
            // un processus ne peut pas dépasser ce qu'utilise l'adaptateur.
            if s.gpu_mem_total > 0 {
                for v in s.gpu_mem.values_mut() {
                    *v = (*v).min(s.gpu_mem_total);
                }
            }
        }
        s
    }
}

impl Drop for Pdh {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}
