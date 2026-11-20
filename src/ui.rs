//! Dessin de l'interface (mêmes composants que l'Explorateur : barre d'outils, volet latéral,
//! liste virtualisée, barre d'état, palette, fenêtres modales).
use crate::app::{human_bytes, human_rate, pct, Action, App, Col, Confirm, Hist, View};
use crate::hunter::{self, Fix, Kind};
use crate::sampler::{Key, Proc};
use crate::sys::{self, Prio};
use egui::{pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Rect, RichText, Sense, Stroke, Ui};
use socle::icons::{self, Icon, Look};
use socle::ui::Command;
use std::sync::Arc;

/// Petite icône vectorielle dans un rectangle (couleur du thème).
fn ico(ui: &Ui, center: egui::Pos2, size: f32, icon: Icon, c: Color32) {
    icons::paint(ui.painter(), Rect::from_center_size(center, vec2(size, size)), icon, c);
}

/// Libellé précédé d'une icône (remplace les emoji dans les labels).
fn icon_label(ui: &mut Ui, icon: Icon, text: impl Into<String>, size: f32, color: Color32) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (r, _) = ui.allocate_exact_size(vec2(size + 2.0, size + 2.0), Sense::hover());
        icons::paint(ui.painter(), r, icon, color);
        ui.label(RichText::new(text.into()).size(size).color(color))
    })
    .inner
}

const ROW_H: f32 = 28.0;

fn trunc(ui: &Ui, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(8.0));
    ui.painter().layout_job(job)
}

fn text_left(ui: &Ui, rect: Rect, x: f32, text: &str, font: FontId, color: Color32, max_w: f32) -> f32 {
    let g = trunc(ui, text, font, color, max_w);
    let w = g.size().x;
    let y = rect.center().y - g.size().y / 2.0;
    ui.painter().galley(pos2(x, y), g, color);
    w
}

fn text_right(ui: &Ui, rect: Rect, right: f32, text: &str, font: FontId, color: Color32) {
    ui.painter().text(pos2(right, rect.center().y), Align2::RIGHT_CENTER, text, font, color);
}

fn section(ui: &mut Ui, p: &socle::theme::Palette, t: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(t.to_uppercase()).size(11.0).color(p.muted).strong());
    ui.add_space(2.0);
}

/// Courbe d'historique remplie (aire sous la courbe en maillage, ligne par-dessus).
fn graph(ui: &Ui, rect: Rect, h: &Hist, max: f32, color: Color32, fill: bool) {
    let n = h.0.len();
    if n < 2 {
        return;
    }
    let max = max.max(1e-6);
    let step = rect.width() / (crate::app::HIST - 1) as f32;
    let x0 = rect.right() - (n - 1) as f32 * step;
    let pts: Vec<egui::Pos2> = h
        .0
        .iter()
        .enumerate()
        .map(|(i, v)| pos2(x0 + i as f32 * step, rect.bottom() - (v / max).clamp(0.0, 1.0) * rect.height()))
        .collect();
    if fill {
        let mut mesh = egui::Mesh::default();
        let fc = color.gamma_multiply(0.22);
        for (i, pt) in pts.iter().enumerate() {
            mesh.colored_vertex(*pt, fc);
            mesh.colored_vertex(pos2(pt.x, rect.bottom()), fc);
            if i > 0 {
                let b = (i as u32) * 2;
                mesh.add_triangle(b - 2, b - 1, b);
                mesh.add_triangle(b - 1, b + 1, b);
            }
        }
        ui.painter().add(egui::Shape::mesh(mesh));
    }
    ui.painter().add(egui::Shape::line(pts, Stroke::new(1.5, color)));
}

/// Intensité 0..1 pour colorer les cellules (comme le Gestionnaire des tâches).
fn heat(col: Col, p: &Proc, agg: Option<&crate::app::Agg>, ncpu: u32) -> f32 {
    let _ = ncpu;
    let v = match col {
        Col::Cpu => agg.map(|a| a.cpu).unwrap_or(p.cpu) / 50.0,
        Col::Mem => (agg.map(|a| a.mem).unwrap_or(p.mem) as f32 / (1536.0 * 1048576.0)).powf(0.8),
        Col::Disk => (agg.map(|a| a.disk).unwrap_or(p.disk) as f32 / (50.0 * 1048576.0)).sqrt(),
        Col::Gpu => agg.map(|a| a.gpu).unwrap_or(p.gpu) / 60.0,
        Col::GpuMem => (agg.map(|a| a.gpu_mem).unwrap_or(p.gpu_mem) as f32 / (2048.0 * 1048576.0)).sqrt(),
        _ => 0.0,
    };
    v.clamp(0.0, 1.0)
}

impl App {
    pub fn draw(&mut self, ui: &mut Ui) {
        if self.rows_dirty {
            self.rebuild_rows();
        }
        let p = self.pal;
        self.draw_toolbar(ui);
        self.draw_status(ui);
        self.draw_sidebar(ui);
        let view = self.s.view;
        if view == View::Procs && self.s.details && self.cursor.is_some() {
            egui::Panel::right("details")
                .resizable(true)
                .default_size(330.0)
                .min_size(260.0)
                .frame(Frame::new().fill(p.side).inner_margin(Margin::same(14)))
                .show(ui, |ui| self.draw_details(ui));
        }
        egui::CentralPanel::default().frame(Frame::new().fill(p.surface)).show(ui, |ui| match view {
            View::Procs => self.draw_procs(ui),
            View::Hunter => self.draw_hunter(ui),
            View::Perf => self.draw_perf(ui),
            View::Startup => self.draw_startup(ui),
        });
        self.draw_confirm(ui);
        self.draw_game(ui);
        self.draw_palette(ui);
        self.draw_help(ui);
    }

    // ---------- Barre d'outils ----------
    fn draw_toolbar(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let ghost_ram: u64 = self.ghosts.iter().map(|g| g.ram).sum();
        egui::Panel::top("toolbar").frame(Frame::new().fill(p.chrome).inner_margin(Margin::symmetric(12, 8))).show(ui, |ui| {
            ui.horizontal(|ui| {
                // vues, en contrôle segmenté (socle)
                let hunt = if self.ghosts.is_empty() { "Chasseur".to_string() } else { format!("Chasseur · {}", human_bytes(ghost_ram)) };
                let views = [View::Procs, View::Hunter, View::Perf, View::Startup];
                let mut cur = views.iter().position(|v| *v == self.s.view).unwrap_or(0);
                if socle::ui::segmented(
                    ui,
                    &p,
                    &mut cur,
                    &[(Some(Icon::ListView), "Processus"), (Some(Icon::Ghost), hunt.as_str()), (Some(Icon::Chart), "Performances"), (Some(Icon::Power), "Démarrage")],
                ) {
                    self.s.view = views[cur];
                }
                if self.s.view == View::Procs {
                    ui.add_space(6.0);
                    self.draw_search(ui);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let theme_icon = if self.s.dark { Icon::Sun } else { Icon::Moon };
                    if icons::button(ui, theme_icon, 32.0, Look::tool(&p), if self.s.dark { "Thème clair" } else { "Thème sombre" }).clicked() {
                        let d = !self.s.dark;
                        self.set_dark(d);
                    }
                    self.draw_more_menu(ui);
                    if self.game_active {
                        if icons::wide(ui, Icon::Gamepad, "Mode jeu actif · Quitter", "", 210.0, Some((p.live, p.on_accent)), &p)
                            .on_hover_text("Restaure le plan d’alimentation et réveille les apps mises en veille")
                            .clicked()
                        {
                            self.actions.push(Action::GameExit);
                        }
                    } else if icons::wide(ui, Icon::Gamepad, "Avant de jouer", "", 158.0, Some((p.accent, p.on_accent)), &p)
                        .on_hover_text("Chasse les fantômes sûrs, met en veille les apps en fond, plan Performances élevées (Ctrl+G)")
                        .clicked()
                    {
                        self.actions.push(Action::GameMode);
                    }
                    if self.s.view == View::Procs {
                        let tree = self.s.tree;
                        let look = |on: bool| if on { Look::active(&p) } else { Look::tool(&p) };
                        if icons::button(ui, Icon::ListView, 32.0, look(!tree), "Liste à plat (Ctrl+T)").clicked() {
                            self.s.tree = false;
                            self.rows_dirty = true;
                        }
                        if icons::button(ui, Icon::Tree, 32.0, look(tree), "Arbre parent / enfants (Ctrl+T)").clicked() {
                            self.s.tree = true;
                            self.rows_dirty = true;
                        }
                    }
                });
            });
        });
    }

    fn draw_search(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let id = egui::Id::new("search");
        if self.focus_search {
            ui.memory_mut(|m| m.request_focus(id));
            self.focus_search = false;
        }
        let before = self.filter.clone();
        // Autocomplétion floue (socle) : historique + noms, descriptions et ports en cours
        let r = socle::ui::autocomplete_field(ui, &p, id, &mut self.search_field, &self.search_ac, &mut self.filter, "Rechercher — nom, PID, :port (Ctrl+F)", 280.0);
        if self.filter != before {
            self.rows_dirty = true;
        }
        if let Some(t) = &r.submitted {
            if !t.trim().is_empty() {
                self.search_ac.history.record(t.trim());
                self.s.search_history = self.search_ac.history.clone();
            }
            // Entrée : on passe à la liste, sur le premier résultat
            r.response.surrender_focus();
            self.rebuild_rows();
            if let Some(snap) = &self.snap {
                if let Some(row) = self.rows.first() {
                    let k = snap.procs[row.idx].key;
                    self.cursor = Some(k);
                    self.anchor = Some(k);
                    self.selected = [k].into_iter().collect();
                    self.scroll_to_cursor = true;
                }
            }
        }
        if r.response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.filter.clear();
            self.rows_dirty = true;
            r.response.surrender_focus();
        }
    }

    fn draw_more_menu(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let more = icons::button(ui, Icon::More, 32.0, Look::tool(&p), "Plus d’options");
        egui::Popup::menu(&more).show(|ui| {
            ui.set_min_width(300.0);
            if ui.button("Palette de commandes…   Ctrl+K").clicked() {
                self.palette.open();
                ui.close();
            }
            ui.separator();
            ui.label(RichText::new("Rafraîchissement").color(p.muted).size(12.0));
            ui.horizontal(|ui| {
                for (ms, l) in [(500u64, "0,5 s"), (1000, "1 s"), (2000, "2 s"), (5000, "5 s")] {
                    if ui.selectable_label(self.s.interval_ms == ms, l).clicked() {
                        self.set_interval(ms);
                    }
                }
            });
            let mut gpu = self.s.gpu;
            if ui.checkbox(&mut gpu, "Mesurer le GPU par processus").on_hover_text("Compteurs « GPU Engine » de Windows. Désactiver économise un peu de CPU.").changed() {
                self.set_gpu(gpu);
            }
            if ui.checkbox(&mut self.s.hide_system, "Masquer les processus Windows et services").changed() {
                self.rows_dirty = true;
            }
            ui.checkbox(&mut self.s.details, "Volet de détails");
            ui.checkbox(&mut self.s.confirm_kill, "Confirmer avant de terminer un processus");
            ui.menu_button("Colonnes", |ui| {
                for c in Col::OPTIONAL {
                    let mut on = self.s.cols.contains(&c);
                    if ui.checkbox(&mut on, c.label()).changed() {
                        if on {
                            self.s.cols.push(c);
                            self.s.cols.sort_by_key(|x| Col::OPTIONAL.iter().position(|y| y == x));
                        } else {
                            self.s.cols.retain(|x| *x != c);
                        }
                    }
                }
            });
            ui.separator();
            let mut tm = self.taskmgr;
            if ui
                .checkbox(&mut tm, "Remplacer le Gestionnaire des tâches (Ctrl+Maj+Échap)")
                .on_hover_text("Windows ouvrira Vigie à la place de taskmgr.exe. Décocher rend la main au Gestionnaire. Demande l’admin une fois.")
                .changed()
            {
                self.actions.push(Action::TaskmgrToggle);
            }
            if !self.elevated && ui.button("Relancer en administrateur").on_hover_text("Voir et gérer tous les processus, vider la mémoire en veille").clicked() {
                self.actions.push(Action::Elevate);
                ui.close();
            }
            ui.separator();
            if ui.button("Raccourcis clavier   F1").clicked() {
                self.help = true;
                ui.close();
            }
        });
    }

    // ---------- Volet latéral : jauges système ----------
    fn draw_sidebar(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::left("side")
            .resizable(true)
            .default_size(self.s.side_w)
            .min_size(180.0)
            .max_size(320.0)
            .frame(Frame::new().fill(p.side).inner_margin(Margin::symmetric(10, 14)))
            .show(ui, |ui| {
                self.s.side_w = ui.available_width() + 20.0;
                let Some(snap) = self.snap.clone() else {
                    ui.label(RichText::new("Mesure…").color(p.muted));
                    return;
                };
                let t = &snap.totals;
                section(ui, &p, "Système");
                let mem_frac = t.mem_used as f32 / t.mem_total.max(1) as f32 * 100.0;
                let disk = t.disk_read + t.disk_write;
                let net = t.net_rx + t.net_tx;
                let disk_max = self.hist.disk.max().max(1024.0 * 1024.0);
                let net_max = self.hist.net.max().max(256.0 * 1024.0);
                let gauges: [(&str, String, &Hist, f32, Color32); 5] = [
                    ("Processeur", pct(t.cpu), &self.hist.cpu, 100.0, p.accent),
                    ("Mémoire", format!("{} · {:.0} %", human_bytes(t.mem_used), mem_frac), &self.hist.mem, 100.0, p.add),
                    ("GPU", if self.s.gpu { pct(t.gpu) } else { "désactivé".into() }, &self.hist.gpu, 100.0, p.live),
                    ("Disque", human_rate(disk), &self.hist.disk, disk_max, p.del),
                    ("Réseau", human_rate(net), &self.hist.net, net_max, p.accent),
                ];
                for (label, val, h, max, color) in gauges {
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::click());
                    let on = self.s.view == View::Perf;
                    let bg = if r.hovered() { p.hover_bg } else if on { p.soft } else { Color32::TRANSPARENT };
                    ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
                    ui.painter().text(pos2(rect.left() + 10.0, rect.top() + 12.0), Align2::LEFT_CENTER, label, FontId::proportional(12.5), p.text);
                    ui.painter().text(pos2(rect.right() - 10.0, rect.top() + 12.0), Align2::RIGHT_CENTER, &val, FontId::proportional(12.0), p.muted);
                    let g = Rect::from_min_max(pos2(rect.left() + 10.0, rect.top() + 24.0), pos2(rect.right() - 10.0, rect.bottom() - 5.0));
                    ui.painter().rect_filled(g, CornerRadius::same(3), p.code);
                    graph(ui, g.shrink(1.0), h, max, color, true);
                    if r.on_hover_text("Voir les graphiques (Ctrl+Maj+G)").clicked() {
                        self.s.view = View::Perf;
                    }
                }
                ui.add_space(8.0);
                section(ui, &p, "Chasseur de RAM");
                let total: u64 = self.ghosts.iter().map(|g| g.ram).sum();
                let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
                let bg = if r.hovered() { p.hover_bg } else if self.s.view == View::Hunter { p.soft } else { Color32::TRANSPARENT };
                ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
                let (big, small, c) = if self.ghosts.is_empty() {
                    ("Rien à chasser".to_string(), "aucun fantôme détecté".to_string(), p.add)
                } else {
                    (format!("{} récupérables", human_bytes(total)), format!("{} fantôme(s) · {} sûr(s)", self.ghosts.len(), self.ghosts.iter().filter(|g| g.safe).count()), p.live)
                };
                ico(ui, pos2(rect.left() + 18.0, rect.top() + 16.0), 16.0, Icon::Ghost, c);
                ui.painter().text(pos2(rect.left() + 32.0, rect.top() + 16.0), Align2::LEFT_CENTER, big, FontId::proportional(13.5), c);
                ui.painter().text(pos2(rect.left() + 10.0, rect.top() + 36.0), Align2::LEFT_CENTER, small, FontId::proportional(11.5), p.muted);
                if r.clicked() {
                    self.s.view = View::Hunter;
                }
                // V2 : sieste de projet
                if !self.projects.is_empty() {
                    ui.add_space(8.0);
                    section(ui, &p, "Projets");
                    for pr in self.projects.clone().iter().take(6) {
                        let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
                        if r.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(7), p.hover_bg);
                        }
                        let asleep = pr.asleep();
                        let c = if asleep { p.muted } else { p.text };
                        ui.painter().text(pos2(rect.left() + 10.0, rect.top() + 12.0), Align2::LEFT_CENTER, &pr.name, FontId::proportional(13.0), c);
                        let sub = if asleep { format!("endormi · {} processus", pr.keys.len()) } else { format!("{} processus · {}", pr.keys.len(), human_bytes(pr.ram)) };
                        ui.painter().text(pos2(rect.left() + 10.0, rect.top() + 29.0), Align2::LEFT_CENTER, sub, FontId::proportional(11.0), p.muted);
                        let br = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(28.0, 28.0));
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
                        let (icon, tip) = if asleep { (Icon::Bolt, "Réveiller le projet") } else { (Icon::Sleep, "Endormir le projet : suspend ses processus et libère leur RAM, sans rien fermer") };
                        if icons::button(&mut child, icon, 28.0, Look::tool(&p), tip).clicked() {
                            self.actions.push(if asleep { Action::WakeProject(pr.dir.clone()) } else { Action::SleepProject(pr.dir.clone()) });
                        }
                    }
                }
                ui.add_space(8.0);
                section(ui, &p, "Session");
                let txt = format!(
                    "{} processus · {} threads\n{} handles · {} cœurs logiques",
                    snap.procs.len(),
                    fmt_int(t.threads as u64),
                    fmt_int(t.handles as u64),
                    snap.ncpu
                );
                ui.label(RichText::new(txt).size(11.5).color(p.muted));
                if self.elevated {
                    icon_label(ui, Icon::Shield, "administrateur", 11.5, p.add);
                } else if icons::wide(ui, Icon::Shield, "Relancer en admin", "", 170.0, None, &p)
                    .on_hover_text("Sans admin : mémoire en veille, services et processus protégés inaccessibles")
                    .clicked()
                {
                    self.actions.push(Action::Elevate);
                }
            });
    }

    // ---------- Barre d'état ----------
    fn draw_status(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::bottom("status").frame(Frame::new().fill(p.chrome).inner_margin(Margin::symmetric(14, 5))).show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(snap) = &self.snap {
                    let t = &snap.totals;
                    let mut txt = format!(
                        "{} processus · CPU {} · RAM {} / {} · engagée {} / {}",
                        snap.procs.len(),
                        pct(t.cpu),
                        human_bytes(t.mem_used),
                        human_bytes(t.mem_total),
                        human_bytes(t.commit),
                        human_bytes(t.commit_limit)
                    );
                    if let Some(sb) = t.standby {
                        txt += &format!(" · en veille {}", human_bytes(sb));
                    }
                    if !self.selected.is_empty() {
                        let mem: u64 = self.selected.iter().filter_map(|k| self.proc(*k)).map(|p| p.mem).sum();
                        txt += &format!(" · {} sélectionné(s), {}", self.selected.len(), human_bytes(mem));
                    }
                    ui.label(RichText::new(txt).color(p.muted).size(12.0));
                }
                self.toasts.ui(ui, &p);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(snap) = &self.snap {
                        ui.label(RichText::new(format!("mesure {:.1} ms · image {:.0} ms", snap.cost_ms, self.last_frame_ms)).color(p.muted).size(12.0))
                            .on_hover_text(format!("Coût d’une mesure (thread d’arrière-plan) et d’une image d’interface\n{}", snap.parts));
                    }
                    if self.freeze {
                        icon_label(ui, Icon::Pause, "ordre figé", 12.0, p.live).on_hover_text("Tant que la souris est sur la liste, les lignes ne changent pas de place");
                    }
                });
            });
        });
    }

    // ---------- Liste des processus ----------
    fn sort_header(&mut self, ui: &mut Ui, rect: Rect, col: Col, right: bool) {
        let p = self.pal;
        let r = ui.interact(rect, ui.id().with(("hdr", col.label())), Sense::click());
        let txt = col.label().to_uppercase();
        let color = if r.hovered() || self.s.sort == col { p.text } else { p.muted };
        let chevron = (self.s.sort == col).then(|| if self.s.asc { Icon::ChevronUp } else { Icon::ChevronDown });
        if right {
            let x = rect.right() - 6.0 - if chevron.is_some() { 14.0 } else { 0.0 };
            text_right(ui, rect, x, &txt, FontId::proportional(11.0), color);
            if let Some(ch) = chevron {
                ico(ui, pos2(rect.right() - 11.0, rect.center().y), 11.0, ch, color);
            }
        } else {
            let w = text_left(ui, rect, rect.left(), &txt, FontId::proportional(11.0), color, rect.width() - 16.0);
            if let Some(ch) = chevron {
                ico(ui, pos2(rect.left() + w + 9.0, rect.center().y), 11.0, ch, color);
            }
        }
        if r.clicked() {
            if self.s.sort == col {
                self.s.asc = !self.s.asc;
            } else {
                self.s.sort = col;
                self.s.asc = col == Col::Name || col == Col::Pid;
            }
            self.freeze = false;
            self.rank.clear();
            self.rows_dirty = true;
        }
    }

    fn draw_procs(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let Some(snap) = self.snap.clone() else {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("Première mesure…").color(p.muted)));
            return;
        };
        let full = ui.available_rect_before_wrap();
        // colonnes de droite à gauche
        let mut cols: Vec<(Col, f32, f32)> = Vec::new(); // (col, x_gauche, x_droite)
        let mut x = full.right() - 12.0;
        let mut avail = full.width() - 260.0;
        for &c in self.s.cols.iter().rev() {
            let w = c.width();
            if avail < w {
                continue;
            }
            avail -= w;
            cols.push((c, x - w, x));
            x -= w;
        }
        cols.reverse();
        let name_right = x - 8.0;
        let header = Rect::from_min_size(full.min, vec2(full.width(), 40.0));
        ui.painter().line_segment([header.left_bottom(), header.right_bottom()], Stroke::new(1.0, p.border));
        let hy = |x0: f32, x1: f32| Rect::from_min_max(pos2(x0, header.top() + 14.0), pos2(x1, header.bottom()));
        self.sort_header(ui, hy(full.left() + 16.0, name_right), Col::Name, false);
        // totaux dans l'en-tête, comme le Gestionnaire (« 23 % » au-dessus de « Processeur »)
        let t = &snap.totals;
        for &(c, x0, x1) in &cols {
            let right = !matches!(c, Col::Status | Col::Ports);
            let r = if c == Col::Ports { hy(x0 + 14.0, x1) } else if c == Col::Status { hy(x0 + 6.0, x1) } else { hy(x0, x1) };
            self.sort_header(ui, r, c, right);
            let tot = match c {
                Col::Cpu => Some(pct(t.cpu)),
                Col::Mem => Some(format!("{:.0} %", t.mem_used as f32 / t.mem_total.max(1) as f32 * 100.0)),
                Col::Gpu if self.s.gpu => Some(pct(t.gpu)),
                _ => None,
            };
            if let Some(tot) = tot {
                ui.painter().text(pos2(x1 - 6.0, header.top() + 4.0), Align2::RIGHT_TOP, tot, FontId::proportional(13.0), p.accent);
            }
        }
        ui.advance_cursor_after_rect(header);

        let n = self.rows.len();
        if n == 0 {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("Aucun processus ne correspond").color(p.muted).size(15.0)));
            return;
        }
        let row_full = ROW_H + ui.spacing().item_spacing.y;
        let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("procs");
        let list_rect = Rect::from_min_max(pos2(full.left(), header.bottom()), full.max);
        if self.scroll_to_cursor {
            self.scroll_to_cursor = false;
            if let Some(i) = self.cursor.and_then(|c| self.rows.iter().position(|r| snap.procs[r.idx].key == c)) {
                let off = ui.ctx().data(|d| d.get_temp::<f32>(egui::Id::new("procs_off"))).unwrap_or(0.0);
                let vh = list_rect.height();
                let top = i as f32 * row_full;
                if top < off {
                    sa = sa.vertical_scroll_offset(top);
                } else if top + row_full > off + vh {
                    sa = sa.vertical_scroll_offset(top + row_full - vh);
                }
            }
        }
        // ordre figé tant que la souris survole la liste
        let hovering = ui.rect_contains_pointer(list_rect) && !self.palette.is_open();
        if hovering != self.freeze {
            self.freeze = hovering;
            if !hovering {
                self.rows_dirty = true;
            }
        }
        let ghosts: std::collections::HashMap<Key, &str> = self.ghosts.iter().map(|g| (g.key, g.reason.as_str())).collect();
        let ghosts: std::collections::HashMap<Key, String> = ghosts.into_iter().map(|(k, v)| (k, v.to_string())).collect();
        let rows = self.rows.clone();
        let mut acts: Vec<Action> = Vec::new();
        let mut toggle: Option<Key> = None;
        let mut click: Option<(Key, egui::Modifiers)> = None;
        let mut dbl: Option<Key> = None;
        let mut ctx_menu_key: Option<Key> = None;
        let out = sa.show_rows(ui, ROW_H, n, |ui, range| {
            for ri in range {
                let row = &rows[ri];
                let pr = &snap.procs[row.idx];
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let selected = self.selected.contains(&pr.key);
                let is_cursor = self.cursor == Some(pr.key);
                let bg = if selected { p.sel_bg } else if resp.hovered() { p.hover_bg } else { Color32::TRANSPARENT };
                ui.painter().rect_filled(rect.shrink2(vec2(6.0, 0.0)), CornerRadius::same(6), bg);
                if is_cursor && !selected {
                    ui.painter().rect_stroke(rect.shrink2(vec2(6.0, 0.5)), CornerRadius::same(6), Stroke::new(1.0, p.accent.gamma_multiply(0.6)), egui::StrokeKind::Inside);
                }
                // cellules « chaudes »
                for &(c, x0, x1) in &cols {
                    let h = heat(c, pr, row.agg.as_ref(), snap.ncpu);
                    if h > 0.06 {
                        let cell = Rect::from_min_max(pos2(x0 + 2.0, rect.top() + 1.0), pos2(x1, rect.bottom() - 1.0));
                        ui.painter().rect_filled(cell, CornerRadius::same(4), p.live.gamma_multiply(0.05 + h * 0.4));
                    }
                }
                // arbre
                let indent = row.depth as f32 * 16.0;
                let mut xn = rect.left() + 14.0 + indent;
                if self.s.tree {
                    if row.has_kids {
                        let ar = Rect::from_center_size(pos2(xn + 7.0, rect.center().y), vec2(18.0, 22.0));
                        let ar_r = ui.interact(ar, ui.id().with(("tw", pr.key)), Sense::click());
                        let c = if ar_r.hovered() { p.text } else { p.muted };
                        ico(ui, ar.center(), 13.0, if row.open { Icon::ChevronDown } else { Icon::ChevronRight }, c);
                        if ar_r.clicked() {
                            toggle = Some(pr.key);
                        }
                    }
                    xn += 18.0;
                }
                // icône
                let ic = Rect::from_center_size(pos2(xn + 8.0, rect.center().y), vec2(16.0, 16.0));
                let mut desc: Option<String> = None;
                let mut drew = false;
                if let Some(path) = pr.path.clone() {
                    if let Some(e) = self.icon(&path) {
                        desc = e.desc.clone();
                        if let Some(tex) = &e.tex {
                            ui.painter().image(tex.id(), ic, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            drew = true;
                        }
                    }
                }
                if !drew {
                    let g = if pr.session == 0 || pr.key.pid <= 4 { Icon::Settings } else { Icon::File };
                    icons::paint(ui.painter(), ic.shrink(1.0), g, p.muted);
                }
                xn += 24.0;
                // nom : description lisible, puis l'exe en gris
                let name_max = name_right - xn;
                let main = desc.as_deref().filter(|d| !d.is_empty() && d.len() < 60).unwrap_or(&pr.name);
                let color = if pr.suspended { p.muted } else { p.text };
                let w = text_left(ui, rect, xn, main, FontId::proportional(13.5), color, name_max);
                let mut xe = xn + w + 8.0;
                if main != &*pr.name && xe < name_right - 40.0 {
                    let w2 = text_left(ui, rect, xe, &pr.name, FontId::proportional(11.5), p.muted, name_right - xe);
                    xe += w2 + 8.0;
                }
                if let Some(a) = &row.agg {
                    if a.count > 1 && xe < name_right - 30.0 {
                        let badge = format!("{}", a.count);
                        let bw = 12.0 + badge.len() as f32 * 7.0;
                        let br = Rect::from_min_size(pos2(xe, rect.center().y - 9.0), vec2(bw, 18.0));
                        ui.painter().rect_filled(br, CornerRadius::same(9), p.soft);
                        ui.painter().text(br.center(), Align2::CENTER_CENTER, badge, FontId::proportional(11.0), p.muted);
                    }
                }
                // colonnes
                let f = FontId::proportional(12.5);
                for &(c, x0, x1) in &cols {
                    let a = row.agg.as_ref();
                    match c {
                        Col::Pid => text_right(ui, rect, x1 - 6.0, &pr.key.pid.to_string(), FontId::monospace(12.0), p.muted),
                        Col::Cpu => text_right(ui, rect, x1 - 6.0, &pct(a.map(|a| a.cpu).unwrap_or(pr.cpu)), f.clone(), p.text),
                        Col::Mem => text_right(ui, rect, x1 - 6.0, &human_bytes(a.map(|a| a.mem).unwrap_or(pr.mem)), f.clone(), p.text),
                        Col::Disk => text_right(ui, rect, x1 - 6.0, &human_rate(a.map(|a| a.disk).unwrap_or(pr.disk)), f.clone(), p.text),
                        Col::Gpu => text_right(ui, rect, x1 - 6.0, &pct(a.map(|a| a.gpu).unwrap_or(pr.gpu)), f.clone(), p.text),
                        Col::GpuMem => {
                            let v = a.map(|a| a.gpu_mem).unwrap_or(pr.gpu_mem);
                            text_right(ui, rect, x1 - 6.0, &if v > 0 { human_bytes(v) } else { String::new() }, f.clone(), p.text)
                        }
                        Col::Threads => text_right(ui, rect, x1 - 6.0, &pr.threads.to_string(), f.clone(), p.muted),
                        Col::Ports => {
                            let s: Vec<String> = pr.ports.iter().filter(|x| x.tcp).map(|x| format!(":{}", x.port)).take(3).collect();
                            text_left(ui, rect, x0 + 14.0, &s.join(" "), FontId::monospace(11.5), p.accent, x1 - x0 - 16.0);
                        }
                        Col::Status => {
                            let (s, c, icon) = if pr.suspended {
                                ("Suspendu", p.del, Some(Icon::Pause))
                            } else if ghosts.contains_key(&pr.key) {
                                ("Fantôme", p.live, Some(Icon::Ghost))
                            } else if pr.window.is_some() {
                                ("App", p.text, None)
                            } else if pr.session == 0 || pr.key.pid <= 4 {
                                ("Système", p.muted, None)
                            } else {
                                ("Fond", p.muted, None)
                            };
                            let prio = Prio::from_base(pr.base_prio);
                            let s = if prio != Prio::Normal && pr.key.pid > 4 { format!("{s} · {}", prio.short()) } else { s.to_string() };
                            let mut x = x0 + 6.0;
                            if let Some(ic) = icon {
                                ico(ui, pos2(x + 6.0, rect.center().y), 12.0, ic, c);
                                x += 16.0;
                            }
                            text_left(ui, rect, x, &s, FontId::proportional(12.0), c, x1 - x - 2.0);
                        }
                        Col::Name => {}
                    }
                }
                let resp = resp.on_hover_ui(|ui| {
                    ui.set_max_width(420.0);
                    ui.label(RichText::new(format!("{} · PID {}", pr.name, pr.key.pid)).strong());
                    if let Some(w) = &pr.window {
                        ui.label(format!("Fenêtre : {w}"));
                    }
                    if let Some(g) = ghosts.get(&pr.key) {
                        icon_label(ui, Icon::Ghost, g.as_str(), 12.5, p.live);
                    }
                    if let Some(path) = &pr.path {
                        ui.label(RichText::new(&**path).color(p.muted).size(11.5));
                    }
                });
                if resp.clicked() {
                    click = Some((pr.key, ui.input(|i| i.modifiers)));
                }
                if resp.double_clicked() {
                    dbl = Some(pr.key);
                }
                if resp.secondary_clicked() && !self.selected.contains(&pr.key) {
                    ctx_menu_key = Some(pr.key);
                }
                let targets = if self.selected.contains(&pr.key) { self.targets() } else { vec![pr.key] };
                resp.context_menu(|ui| self.proc_menu(ui, &targets, &mut acts));
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("procs_off"), out.state.offset().y));
        if let Some(k) = ctx_menu_key {
            self.cursor = Some(k);
            self.selected = [k].into_iter().collect();
        }
        if let Some((k, m)) = click {
            if m.command {
                if !self.selected.remove(&k) {
                    self.selected.insert(k);
                }
                self.cursor = Some(k);
                self.anchor = Some(k);
            } else if m.shift {
                let keys = self.row_keys();
                let a = self.anchor.and_then(|a| keys.iter().position(|x| *x == a));
                let b = keys.iter().position(|x| *x == k);
                if let (Some(a), Some(b)) = (a, b) {
                    self.selected = keys[a.min(b)..=a.max(b)].iter().copied().collect();
                }
                self.cursor = Some(k);
            } else {
                self.selected = [k].into_iter().collect();
                self.cursor = Some(k);
                self.anchor = Some(k);
            }
        }
        if let Some(k) = dbl {
            if self.proc(k).map(|p| p.window.is_some()).unwrap_or(false) {
                acts.push(Action::Focus(k));
            } else {
                self.s.details = true;
            }
        }
        if let Some(k) = toggle {
            if !self.collapsed.remove(&k) {
                self.collapsed.insert(k);
            }
            self.rows_dirty = true;
        }
        self.actions.extend(acts);
    }

    /// Menu contextuel d'un ou plusieurs processus.
    fn proc_menu(&self, ui: &mut Ui, keys: &[Key], acts: &mut Vec<Action>) {
        let many = keys.len() > 1;
        let first = keys.first().and_then(|k| self.proc(*k));
        ui.set_min_width(250.0);
        if many {
            ui.label(RichText::new(format!("{} processus", keys.len())).color(self.pal.muted).size(12.0));
        }
        if let Some(pr) = first {
            if !many && pr.window.is_some() && ui.button("Basculer vers la fenêtre   Entrée").clicked() {
                acts.push(Action::Focus(pr.key));
                ui.close();
            }
        }
        if ui.button("Fermer proprement").on_hover_text("Demande aux fenêtres de se fermer (comme le bouton ×)").clicked() {
            acts.push(Action::Close(keys.to_vec()));
            ui.close();
        }
        if ui.button(RichText::new("Terminer   Suppr").color(self.pal.del)).clicked() {
            acts.push(Action::Kill(keys.to_vec(), false));
            ui.close();
        }
        if ui.button(RichText::new("Terminer l’arborescence   Maj+Suppr").color(self.pal.del)).clicked() {
            acts.push(Action::Kill(keys.to_vec(), true));
            ui.close();
        }
        let susp = first.map(|p| p.suspended).unwrap_or(false);
        if ui.button("Mettre en veille (libérer sa RAM sans fermer)").on_hover_text("Vide sa plage de travail : la RAM redevient libre, le processus continue").clicked() {
            acts.push(Action::Trim(keys.to_vec()));
            ui.close();
        }
        if ui.button(if susp { "Reprendre   Espace" } else { "Suspendre   Espace" }).clicked() {
            acts.push(Action::Suspend(keys.to_vec(), !susp));
            ui.close();
        }
        ui.menu_button("Priorité", |ui| {
            let cur = first.map(|p| Prio::from_base(p.base_prio));
            for pr in Prio::ALL {
                if ui.selectable_label(cur == Some(pr), pr.label()).clicked() {
                    acts.push(Action::Priority(keys.to_vec(), pr));
                    ui.close();
                }
            }
        });
        ui.separator();
        if let Some(pr) = first {
            if pr.path.is_some() && ui.button("Ouvrir l’emplacement   Ctrl+E").clicked() {
                acts.push(Action::OpenLocation(pr.key));
                ui.close();
            }
            ui.menu_button("Copier", |ui| {
                if ui.button("Nom et PID   Ctrl+C").clicked() {
                    acts.push(Action::Copy(format!("{} ({})", pr.name, pr.key.pid)));
                    ui.close();
                }
                if let Some(path) = &pr.path {
                    if ui.button("Chemin   Ctrl+Maj+C").clicked() {
                        acts.push(Action::Copy(path.to_string()));
                        ui.close();
                    }
                }
                if let Some(cmd) = &pr.cmd {
                    if ui.button("Ligne de commande").clicked() {
                        acts.push(Action::Copy(cmd.to_string()));
                        ui.close();
                    }
                }
            });
            if let Some(par) = pr.parent {
                if ui.button("Aller au parent").clicked() {
                    acts.push(Action::Select(par));
                    ui.close();
                }
            }
            if ui.button("Rechercher en ligne").clicked() {
                acts.push(Action::WebSearch(pr.name.to_string()));
                ui.close();
            }
            if ui.button("Ne jamais proposer au chasseur").on_hover_text(format!("Ajoute « {} » à la liste blanche", pr.name)).clicked() {
                acts.push(Action::Whitelist(pr.name.to_string()));
                ui.close();
            }
        }
    }

    // ---------- Détails ----------
    fn draw_details(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let Some(k) = self.cursor else { return };
        let Some(pr) = self.proc(k).cloned() else { return };
        let desc = pr.path.as_ref().and_then(|pa| self.icons.get(pa)).and_then(|e| e.desc.clone());
        let mut acts = Vec::new();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(tex) = pr.path.as_ref().and_then(|pa| self.icons.get(pa)).and_then(|e| e.tex.clone()) {
                    ui.add(egui::Image::new(&tex).fit_to_exact_size(vec2(28.0, 28.0)));
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new(desc.as_deref().unwrap_or(&pr.name)).size(16.0).strong());
                    ui.label(RichText::new(format!("{} · PID {}", pr.name, pr.key.pid)).color(p.muted).size(12.0));
                });
                ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                    if ui.add(egui::Button::new(RichText::new("×").size(16.0)).frame(false)).on_hover_text("Fermer le volet").clicked() {
                        self.s.details = false;
                    }
                });
            });
            if let Some(g) = self.ghosts.iter().find(|g| g.key == pr.key) {
                ui.add_space(4.0);
                Frame::new().fill(p.live_bg).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    icon_label(ui, Icon::Ghost, format!("{} — {}", g.kind.label(), g.reason), 12.5, p.live);
                });
            }
            ui.add_space(8.0);
            // actions rapides
            ui.horizontal_wrapped(|ui| {
                if pr.window.is_some() && ui.button("Basculer").on_hover_text("Mettre sa fenêtre au premier plan (Entrée)").clicked() {
                    acts.push(Action::Focus(pr.key));
                }
                if ui.add(egui::Button::new(RichText::new("Terminer").color(Color32::WHITE)).fill(p.del)).on_hover_text("Suppr").clicked() {
                    acts.push(Action::Kill(vec![pr.key], false));
                }
                if ui.button("Arbre").on_hover_text("Terminer avec tous ses descendants (Maj+Suppr)").clicked() {
                    acts.push(Action::Kill(vec![pr.key], true));
                }
                if ui.button(if pr.suspended { "Reprendre" } else { "Suspendre" }).clicked() {
                    acts.push(Action::Suspend(vec![pr.key], !pr.suspended));
                }
                if pr.path.is_some() && icons::button(ui, Icon::Folder, 28.0, Look::tool(&p), "Ouvrir l’emplacement (Ctrl+E)").clicked() {
                    acts.push(Action::OpenLocation(pr.key));
                }
            });
            ui.add_space(10.0);
            let (hc, hm) = self.phist.get(&pr.key).cloned().unwrap_or_default();
            let card = |ui: &mut Ui, title: &str, val: String, h: &Hist, max: f32, color: Color32| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).size(12.0).color(p.muted));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new(val).size(12.5).strong()));
                });
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(6), p.code);
                graph(ui, r.shrink(2.0), h, max, color, true);
                ui.add_space(4.0);
            };
            card(ui, "Processeur", pct(pr.cpu), &hc, hc.max().max(5.0), p.accent);
            card(ui, "Mémoire (privée)", human_bytes(pr.mem), &hm, hm.max().max(1.0) * 1.15, p.add);
            ui.add_space(6.0);
            egui::Grid::new("kv").num_columns(2).spacing(vec2(12.0, 5.0)).show(ui, |ui| {
                let kv = |ui: &mut Ui, k: &str, v: String| {
                    ui.label(RichText::new(k).color(p.muted).size(12.0));
                    ui.label(RichText::new(v).size(12.0));
                    ui.end_row();
                };
                kv(ui, "Statut", if pr.suspended { "Suspendu".into() } else if pr.window.is_some() { "App (fenêtre)".into() } else { "Arrière-plan".into() });
                if let Some(w) = &pr.window {
                    kv(ui, "Fenêtre", w.to_string());
                }
                kv(ui, "Lancé il y a", hunter::human_idle(pr.age_secs));
                kv(ui, "Inactif depuis", if pr.idle_secs < 2.0 { "actif".into() } else { hunter::idle_text(&pr) });
                kv(ui, "Mémoire engagée", human_bytes(pr.commit));
                kv(ui, "Plage de travail", human_bytes(pr.ws));
                if pr.gpu_mem > 0 || pr.gpu > 0.0 {
                    kv(ui, "GPU", format!("{} · {}", pct(pr.gpu), human_bytes(pr.gpu_mem)));
                }
                kv(ui, "Disque", human_rate(pr.disk));
                kv(ui, "Threads / handles", format!("{} / {}", pr.threads, fmt_int(pr.handles as u64)));
                kv(ui, "Priorité", Prio::from_base(pr.base_prio).label().into());
                kv(ui, "Session", pr.session.to_string());
            });
            if let Some(par) = pr.parent.and_then(|k| self.proc(k)) {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Parent").color(p.muted).size(12.0));
                    if ui.link(format!("{} ({})", par.name, par.key.pid)).clicked() {
                        acts.push(Action::Select(par.key));
                    }
                });
            } else if pr.key.pid > 4 {
                ui.label(RichText::new(format!("Parent {} disparu (orphelin)", pr.ppid)).color(p.muted).size(12.0));
            }
            if !pr.ports.is_empty() {
                section(ui, &p, "Ports ouverts");
                ui.horizontal_wrapped(|ui| {
                    for x in pr.ports.iter() {
                        let txt = format!("{} {}{}", if x.tcp { "TCP" } else { "UDP" }, x.port, if x.local_only { " (local)" } else { "" });
                        let r = ui.add(egui::Button::new(RichText::new(txt).monospace().size(11.5)).small());
                        if x.tcp && r.on_hover_text("Ouvrir http://localhost:port").clicked() {
                            let _ = std::process::Command::new("explorer.exe").arg(format!("http://localhost:{}", x.port)).spawn();
                        }
                    }
                });
            }
            if let Some(path) = &pr.path {
                section(ui, &p, "Chemin");
                if ui.add(egui::Label::new(RichText::new(&**path).size(11.5).color(p.accent)).sense(Sense::click())).on_hover_text("Ouvrir l’emplacement").clicked() {
                    acts.push(Action::OpenLocation(pr.key));
                }
            }
            if let Some(cmd) = &pr.cmd {
                section(ui, &p, "Ligne de commande");
                let mut c = cmd.to_string();
                ui.add(egui::TextEdit::multiline(&mut c).font(FontId::monospace(11.0)).desired_rows(2).desired_width(f32::INFINITY));
            }
        });
        self.actions.extend(acts);
    }

    // ---------- Chasseur ----------
    fn draw_hunter(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let mut acts = Vec::new();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            Frame::new().inner_margin(Margin::symmetric(24, 18)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let total: u64 = self.ghosts.iter().map(|g| g.ram).sum();
                let sel: Vec<Key> = self.ghosts.iter().filter(|g| self.checked.get(&g.key).copied().unwrap_or(false)).map(|g| g.key).collect();
                let sel_ram: u64 = self.ghosts.iter().filter(|g| sel.contains(&g.key)).map(|g| g.ram).sum();
                // carte principale
                Frame::new().fill(p.side).stroke(Stroke::new(1.0, p.border)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(18)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            if self.ghosts.is_empty() {
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                                    icons::paint(ui.painter(), r, Icon::Ghost, p.add);
                                    ui.label(RichText::new("Aucun fantôme").size(26.0).strong().color(p.add));
                                });
                                ui.label(RichText::new("Rien ne tourne pour rien : pas de serveur de dev oublié, d’orphelin ni d’app cachée gourmande.").color(p.muted));
                            } else {
                                ui.label(RichText::new(format!("{} récupérables", human_bytes(total))).size(28.0).strong().color(p.live));
                                ui.label(
                                    RichText::new(format!(
                                        "{} fantôme(s) · les cibles sûres sont pré-cochées, le reste est à vérifier",
                                        self.ghosts.len()
                                    ))
                                    .color(p.muted),
                                );
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if icons::wide(ui, Icon::Gamepad, "Avant de jouer", "Ctrl+G", 200.0, None, &p).clicked() {
                                acts.push(Action::GameMode);
                            }
                            let label = format!("Nettoyer la sélection · {}", human_bytes(sel_ram));
                            let w = 70.0 + label.chars().count() as f32 * 7.4;
                            if ui
                                .add_enabled_ui(!sel.is_empty(), |ui| icons::wide(ui, Icon::Broom, &label, "", w, Some((p.accent, p.on_accent)), &p))
                                .inner
                                .on_hover_text("Termine chaque processus coché avec ses descendants")
                                .clicked()
                            {
                                acts.push(Action::Clean(sel.clone()));
                            }
                        });
                    });
                });
                ui.add_space(12.0);
                for kind in Kind::ORDER {
                    let list: Vec<crate::hunter::Ghost> = self.ghosts.iter().filter(|g| g.kind == kind).cloned().collect();
                    if list.is_empty() {
                        continue;
                    }
                    let sum: u64 = list.iter().map(|g| g.ram).sum();
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        icons::paint(ui.painter(), r, kind.icon(), p.muted);
                        ui.label(RichText::new(kind.label().to_uppercase()).size(11.5).color(p.muted).strong());
                        ui.label(RichText::new(human_bytes(sum)).size(11.5).color(p.muted));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let all = list.iter().all(|g| self.checked.get(&g.key).copied().unwrap_or(false));
                            if ui.add(egui::Button::new(RichText::new(if all { "tout décocher" } else { "tout cocher" }).size(11.5).color(p.accent)).frame(false)).clicked() {
                                for g in &list {
                                    self.checked.insert(g.key, !all);
                                }
                            }
                        });
                    });
                    for g in &list {
                        let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::click());
                        let on = self.checked.get(&g.key).copied().unwrap_or(false);
                        let bg = if on { p.sel_bg } else if r.hovered() { p.hover_bg } else { Color32::TRANSPARENT };
                        ui.painter().rect_filled(rect, CornerRadius::same(8), bg);
                        // case
                        let cb = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(16.0, 16.0));
                        ui.painter().rect(cb, CornerRadius::same(4), if on { p.accent } else { p.code }, Stroke::new(1.0, if on { p.accent } else { p.border }), egui::StrokeKind::Inside);
                        if on {
                            icons::paint(ui.painter(), cb.shrink(2.0), Icon::Check, p.on_accent);
                        }
                        let mut x = rect.left() + 40.0;
                        if let Some(path) = self.proc(g.key).and_then(|pr| pr.path.clone()) {
                            if let Some(tex) = self.icon(&path).and_then(|e| e.tex.clone()) {
                                ui.painter().image(tex.id(), Rect::from_center_size(pos2(x + 9.0, rect.center().y), vec2(18.0, 18.0)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            }
                        }
                        x += 28.0;
                        let right_w = 330.0;
                        let maxw = rect.right() - right_w - x;
                        let top = Rect::from_min_max(pos2(x, rect.top() + 4.0), pos2(rect.right(), rect.center().y + 2.0));
                        let bot = Rect::from_min_max(pos2(x, rect.center().y), pos2(rect.right(), rect.bottom() - 4.0));
                        let title = if g.count > 1 { format!("{}  ·  {} processus", g.name, g.count) } else { g.name.clone() };
                        let w = text_left(ui, top, x, &title, FontId::proportional(14.0), p.text, maxw);
                        if g.safe {
                            let br = Rect::from_min_size(pos2(x + w + 8.0, top.center().y - 8.0), vec2(36.0, 16.0));
                            ui.painter().rect_filled(br, CornerRadius::same(8), p.add_bg);
                            ui.painter().text(br.center(), Align2::CENTER_CENTER, "sûr", FontId::proportional(10.5), p.add);
                        }
                        text_left(ui, bot, x, &g.reason, FontId::proportional(12.0), p.muted, maxw);
                        ui.painter().text(pos2(rect.right() - 250.0, rect.center().y), Align2::RIGHT_CENTER, human_bytes(g.ram), FontId::proportional(15.0), p.live);
                        // boutons
                        let b1 = Rect::from_min_size(pos2(rect.right() - 236.0, rect.center().y - 14.0), vec2(120.0, 28.0));
                        let b2 = Rect::from_min_size(pos2(rect.right() - 110.0, rect.center().y - 14.0), vec2(100.0, 28.0));
                        let r1 = ui.put(b1, egui::Button::new(RichText::new("Toujours ignorer").size(12.0)));
                        let r1 = r1.on_hover_text(format!("Liste blanche : « {} »", g.rule));
                        if r1.clicked() {
                            acts.push(Action::Whitelist(g.rule.clone()));
                        }
                        let (label, direct) = match &g.fix {
                            Fix::WslShutdown => ("Arrêter WSL", true),
                            Fix::DockerStop(_) => ("docker stop", true),
                            Fix::DockerQuit => ("Quitter Docker", true),
                            Fix::KillTree => ("Voir", false),
                        };
                        let r2 = ui.put(b2, egui::Button::new(RichText::new(label).size(12.0)));
                        if r2.clicked() {
                            if direct {
                                acts.push(Action::Clean(vec![g.key]));
                            } else {
                                acts.push(Action::Select(g.key));
                            }
                        }
                        let r = r.on_hover_ui(|ui| {
                            ui.set_max_width(480.0);
                            ui.label(RichText::new(&g.reason).strong());
                            if let Some(c) = &g.cmd {
                                ui.label(RichText::new(c).monospace().size(11.0).color(p.muted));
                            }
                        });
                        if r.clicked() {
                            self.checked.insert(g.key, !on);
                        }
                    }
                }
                ui.add_space(18.0);
                self.draw_games(ui);
                self.draw_journal(ui);
                ui.add_space(10.0);
                egui::CollapsingHeader::new(RichText::new(format!("Liste blanche ({})", self.s.whitelist.len())).color(p.muted)).id_salt("wl").show(ui, |ui| {
                    if self.s.whitelist.is_empty() {
                        ui.label(RichText::new("Vide. « Toujours ignorer » ajoute un processus ici (par nom, ou nom | fragment de ligne de commande).").color(p.muted).size(12.0));
                    }
                    ui.horizontal_wrapped(|ui| {
                        for r in self.s.whitelist.clone() {
                            if ui.button(format!("{r}  ×")).on_hover_text("Retirer de la liste blanche").clicked() {
                                acts.push(Action::Unwhitelist(r));
                            }
                        }
                    });
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Règles : seuls tes processus (pas les services ni Windows) sont examinés. Claude Code, VS Code, Steam, Discord et l’explorateur ne sont jamais proposés. \
                         « Sûr » = orphelin ou démon conçu pour se relancer, inactif depuis plusieurs minutes.",
                    )
                    .size(11.5)
                    .color(p.muted),
                );
            });
        });
        self.actions.extend(acts);
    }

    // ---------- Performances ----------
    fn draw_perf(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let Some(snap) = self.snap.clone() else { return };
        let t = &snap.totals;
        let mut acts = Vec::new();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            Frame::new().inner_margin(Margin::symmetric(20, 16)).show(ui, |ui| {
                let w = ui.available_width();
                let cols = if w > 900.0 { 2 } else { 1 };
                let cw = (w - 16.0 * (cols as f32 - 1.0)) / cols as f32;
                let disk_max = self.hist.disk.max().max(1024.0 * 1024.0) * 1.15;
                let net_max = self.hist.net.max().max(256.0 * 1024.0) * 1.15;
                use crate::qol::Metric;
                let metric_of = |t: &str| match t {
                    "Processeur" => Some(Metric::Cpu),
                    "Mémoire" => Some(Metric::Mem),
                    "GPU" => Some(Metric::Gpu),
                    "Disque" => Some(Metric::Disk),
                    _ => None,
                };
                let mut pin: Option<(Metric, usize)> = None;
                let cards: Vec<(&str, String, String, &Hist, f32, Color32)> = vec![
                    ("Processeur", pct(t.cpu), format!("{} cœurs logiques · {} threads", snap.ncpu, fmt_int(t.threads as u64)), &self.hist.cpu, 100.0, p.accent),
                    (
                        "Mémoire",
                        format!("{} / {}", human_bytes(t.mem_used), human_bytes(t.mem_total)),
                        format!(
                            "engagée {} / {}{}",
                            human_bytes(t.commit),
                            human_bytes(t.commit_limit),
                            t.standby.map(|s| format!(" · en veille {}", human_bytes(s))).unwrap_or_default()
                        ),
                        &self.hist.mem,
                        100.0,
                        p.add,
                    ),
                    (
                        "GPU",
                        if self.s.gpu { pct(t.gpu) } else { "désactivé".into() },
                        format!("mémoire dédiée utilisée {}", human_bytes(t.gpu_mem)),
                        &self.hist.gpu,
                        100.0,
                        p.live,
                    ),
                    ("Disque", human_rate(t.disk_read + t.disk_write), format!("lecture {} · écriture {}", human_rate(t.disk_read), human_rate(t.disk_write)), &self.hist.disk, disk_max, p.del),
                    ("Réseau", human_rate(t.net_rx + t.net_tx), format!("reçu {} · envoyé {}", human_rate(t.net_rx), human_rate(t.net_tx)), &self.hist.net, net_max, p.accent),
                ];
                let mut it = cards.into_iter().peekable();
                while it.peek().is_some() {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        for _ in 0..cols {
                            let Some((title, big, small, h, max, color)) = it.next() else { break };
                            ui.allocate_ui_with_layout(vec2(cw, 210.0), Layout::top_down(Align::Min), |ui| {
                                Frame::new().fill(p.side).stroke(Stroke::new(1.0, p.border)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(14)).show(ui, |ui| {
                                    ui.set_width(cw - 30.0);
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(title).size(15.0).strong());
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new(big).size(15.0).color(color).strong()));
                                    });
                                    ui.label(RichText::new(small).size(11.5).color(p.muted));
                                    let metric = metric_of(title);
                                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 120.0), if metric.is_some() { Sense::click() } else { Sense::hover() });
                                    ui.painter().rect_filled(r, CornerRadius::same(6), p.code);
                                    for i in 1..4 {
                                        let y = r.top() + r.height() * i as f32 / 4.0;
                                        ui.painter().line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0, p.border.gamma_multiply(0.5)));
                                    }
                                    graph(ui, r.shrink(2.0), h, max, color, true);
                                    // V4 : survol = les 5 coupables de cet instant ; clic = épingler
                                    if let Some(m) = metric {
                                        let pinned = self.bb_pin.filter(|(pm, _)| *pm == m).map(|(_, i)| i);
                                        let hover_i = resp.hover_pos().and_then(|pos| bb_index(r.shrink(2.0), pos.x, self.blackbox.len()));
                                        for i in [pinned, hover_i].into_iter().flatten() {
                                            let x = bb_x(r.shrink(2.0), i, self.blackbox.len());
                                            ui.painter().line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0, p.text.gamma_multiply(0.5)));
                                        }
                                        if let Some(i) = hover_i {
                                            if let Some(mo) = self.blackbox.get(i) {
                                                let list = mo.list(m).to_vec();
                                                let ago = mo.at.elapsed().as_secs();
                                                resp.clone().on_hover_ui_at_pointer(|ui| {
                                                    ui.label(RichText::new(format!("il y a {ago} s — clic pour épingler")).size(11.5).color(p.muted));
                                                    for c in &list {
                                                        ui.label(format!("{}  {}", c.name, fmt_metric(m, c.value)));
                                                    }
                                                    if list.is_empty() {
                                                        ui.label(RichText::new("rien de notable").color(p.muted));
                                                    }
                                                });
                                            }
                                            if resp.clicked() {
                                                pin = Some((m, i));
                                            }
                                        }
                                    }
                                    ui.label(RichText::new(format!("{} s d’historique", crate::app::HIST as u64 * self.s.interval_ms / 1000)).size(10.5).color(p.muted));
                                });
                            });
                        }
                    });
                    ui.add_space(12.0);
                }
                if let Some(pn) = pin {
                    self.bb_pin = if self.bb_pin == Some(pn) { None } else { Some(pn) };
                }
                // V4 : boîte noire épinglée
                if let Some((m, i)) = self.bb_pin {
                    if let Some(mo) = self.blackbox.get(i).cloned() {
                        Frame::new().fill(p.live_bg).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(12)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                                icons::paint(ui.painter(), ir, Icon::Clock, p.live);
                                let what = match m {
                                    Metric::Cpu => "processeur",
                                    Metric::Mem => "mémoire",
                                    Metric::Gpu => "GPU",
                                    Metric::Disk => "disque",
                                };
                                ui.label(RichText::new(format!("Boîte noire · {what} · il y a {} s", mo.at.elapsed().as_secs())).strong().color(p.live));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if icons::button(ui, Icon::Close, 24.0, Look::tool(&p), "Fermer").clicked() {
                                        self.bb_pin = None;
                                    }
                                });
                            });
                            for c in mo.list(m) {
                                ui.horizontal(|ui| {
                                    let alive = self.proc(c.key).is_some();
                                    if alive {
                                        if ui.link(&c.name).clicked() {
                                            acts.push(Action::Select(c.key));
                                        }
                                    } else {
                                        ui.label(RichText::new(format!("{} (terminé depuis)", c.name)).color(p.muted));
                                    }
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new(fmt_metric(m, c.value)).color(p.muted)));
                                });
                            }
                        });
                        ui.add_space(12.0);
                    }
                }
                // plus gros consommateurs : le top 5 de la dernière mesure (déjà calculé par la boîte noire)
                if let Some(mo) = self.blackbox.back().cloned() {
                    ui.columns(2, |c| {
                        for (col, title, m) in [(0usize, "Plus gros en mémoire", Metric::Mem), (1, "Plus gros en processeur", Metric::Cpu)] {
                            let ui = &mut c[col];
                            section(ui, &p, title);
                            for cu in mo.list(m) {
                                ui.horizontal(|ui| {
                                    if ui.link(&cu.name).clicked() {
                                        acts.push(Action::Select(cu.key));
                                    }
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(RichText::new(fmt_metric(m, cu.value)).color(p.muted)));
                                });
                            }
                        }
                    });
                }
            });
        });
        self.actions.extend(acts);
    }

    // ---------- Modales ----------
    fn draw_confirm(&mut self, ui: &mut Ui) {
        let Some(Confirm::Kill { keys, tree, names }) = &self.confirm else { return };
        let p = self.pal;
        let (keys, tree, names) = (keys.clone(), *tree, names.clone());
        let mut answer = None;
        let ctx = ui.ctx().clone();
        let title = if tree { "Terminer l’arborescence ?" } else { "Terminer ?" };
        egui::Window::new(title).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(&ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label(&names);
            ui.label(
                RichText::new(if tree { "Le processus et tous ses descendants seront arrêtés. Les données non enregistrées seront perdues." } else { "Les données non enregistrées seront perdues." })
                    .color(p.muted)
                    .size(12.0),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Terminer   Entrée").color(Color32::WHITE).strong()).fill(p.del)).clicked() {
                    answer = Some(true);
                }
                if ui.button("Annuler   Échap").clicked() {
                    answer = Some(false);
                }
                let mut ask = self.s.confirm_kill;
                if ui.checkbox(&mut ask, "Toujours demander").changed() {
                    self.s.confirm_kill = ask;
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                answer = Some(false);
            }
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                answer = Some(true);
            }
        });
        match answer {
            Some(true) => {
                self.confirm = None;
                self.actions.push(Action::KillNow(keys, tree));
            }
            Some(false) => self.confirm = None,
            None => {}
        }
    }

    fn draw_game(&mut self, ui: &mut Ui) {
        if !self.game_dialog {
            return;
        }
        let p = self.pal;
        let ctx = ui.ctx().clone();
        let safe = self.safe_ghosts();
        let safe_ram: u64 = safe.iter().map(|g| g.ram).sum();
        let standby = self.snap.as_ref().and_then(|s| s.totals.standby);
        let mut go = false;
        let mut open = true;
        egui::Window::new("Mode avant de jouer").open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(&ctx, |ui| {
            ui.set_width(440.0);
            // V3 : « il te manque X Go » pour le dernier jeu joué
            if let Some((name, peak, avail)) = self.last_game() {
                let miss = crate::qol::missing(peak, avail);
                let (txt, c) = if miss == 0 {
                    (format!("{name} : pic {} · {} disponibles — ça passe", human_bytes(peak), human_bytes(avail)), p.add)
                } else {
                    (format!("{name} : pic {} · il te manque {} — coche ce qu’il faut ci-dessous", human_bytes(peak), human_bytes(miss)), p.live)
                };
                icon_label(ui, Icon::Gamepad, txt, 12.5, c);
                ui.add_space(6.0);
            }
            ui.checkbox(&mut self.s.game_kill, format!("Chasser les fantômes sûrs — {} processus, {}", safe.len(), human_bytes(safe_ram)));
            if !safe.is_empty() {
                let names: Vec<String> = safe.iter().take(5).map(|g| g.name.clone()).collect();
                ui.label(RichText::new(format!("    {}", names.join(", "))).color(p.muted).size(11.5));
            }
            let trim = self.trim_candidates();
            let trim_ws: u64 = trim.iter().filter_map(|k| self.proc(*k)).map(|p| p.ws).sum();
            ui.checkbox(&mut self.s.game_trim, format!("Mettre en veille les apps en arrière-plan — {} processus, jusqu’à {}", trim.len(), human_bytes(trim_ws)))
                .on_hover_text("Vide leur RAM sans les fermer (navigateur, Spotify…). Elles restent ouvertes et se réveillent un peu plus lentement. Discord, Steam et la fenêtre au premier plan ne sont pas touchés.");
            ui.add_enabled_ui(self.elevated, |ui| {
                ui.checkbox(
                    &mut self.s.game_purge,
                    match standby {
                        Some(b) => format!("Vider la mémoire en veille — {}", human_bytes(b)),
                        None => "Vider la mémoire en veille".into(),
                    },
                )
                .on_disabled_hover_text("Nécessite les droits administrateur");
            });
            if !self.elevated {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("    nécessite l’admin").color(p.muted).size(11.5));
                    if ui.small_button("Relancer en admin").clicked() {
                        self.actions.push(Action::Elevate);
                    }
                });
            }
            // lu une fois à l'ouverture de la fenêtre (powercfg), pas à chaque image
            let (cur, max) = ui.ctx().memory_mut(|m| {
                m.data.get_temp_mut_or_insert_with(egui::Id::new("power_label"), || {
                    sys::active_power_info().map(|(_, l, m)| (l, m)).unwrap_or_default()
                })
                .clone()
            });
            let label = if max {
                format!("Plan « Performances élevées » — déjà au maximum ({cur}), rien à changer")
            } else {
                format!("Plan « Performances élevées » (actuel : {cur})")
            };
            ui.checkbox(&mut self.s.game_power, label);
            ui.label(RichText::new("    restauré en cliquant « Quitter » ou avec Vigie.exe --apres-jeu").color(p.muted).size(11.5));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Lancer   Entrée").color(p.on_accent).strong()).fill(p.accent)).clicked() {
                    go = true;
                }
                if ui.button("Annuler").clicked() {
                    self.game_dialog = false;
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.game_dialog = false;
            }
        });
        if !open || !self.game_dialog {
            self.game_dialog = false;
            ctx.memory_mut(|m| m.data.remove::<(String, bool)>(egui::Id::new("power_label")));
        }
        if go {
            ctx.memory_mut(|m| m.data.remove::<(String, bool)>(egui::Id::new("power_label")));
            self.game_dialog = false;
            self.run_game_mode();
        }
    }

    fn draw_help(&mut self, ui: &mut Ui) {
        if !self.help {
            return;
        }
        let ctx = ui.ctx().clone();
        let mut open = true;
        egui::Window::new("Raccourcis clavier").open(&mut open).collapsible(false).resizable(false).show(&ctx, |ui| {
            egui::Grid::new("keys").num_columns(2).spacing(vec2(24.0, 4.0)).show(ui, |ui| {
                for (k, v) in [
                    ("Ctrl+K (ou Ctrl+P)", "Palette de commandes"),
                    ("Ctrl+F ou taper", "Rechercher, floue (nom, PID, :port…) ; Tab complète, ↓ suggestions"),
                    ("Ctrl+1 / 2 / 3 / 4", "Processus / Chasseur / Performances / Démarrage (aussi Ctrl+Maj+P / F / G / D)"),
                    ("↑ ↓ PgPréc PgSuiv Début Fin", "Naviguer (Maj : étendre la sélection)"),
                    ("← →", "Replier / déplier (arbre)"),
                    ("Ctrl+T", "Arbre / liste à plat"),
                    ("Entrée", "Basculer vers la fenêtre du processus"),
                    ("Suppr / Maj+Suppr", "Terminer / terminer l’arborescence"),
                    ("Espace", "Suspendre / reprendre"),
                    ("Ctrl+E", "Ouvrir l’emplacement"),
                    ("Ctrl+C / Ctrl+Maj+C", "Copier nom et PID / chemin"),
                    ("Ctrl+A", "Tout sélectionner"),
                    ("Ctrl+G", "Mode avant de jouer"),
                    ("F5", "Mesurer maintenant"),
                    ("Échap", "Effacer la recherche / la sélection"),
                    ("Souris sur la liste", "L’ordre se fige pour viser sans que ça bouge"),
                ] {
                    ui.label(RichText::new(k).monospace().strong());
                    ui.label(v);
                    ui.end_row();
                }
            });
        });
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.help = false;
        }
    }

    /// Commandes de la palette Ctrl+K (construites seulement quand elle est ouverte).
    /// L'id encode l'action : `view:hunter`, `goto:1234:133…`, `killname:node.exe`…
    fn palette_commands(&self) -> Vec<Command> {
        let mut v = vec![
            Command::new("game", "Mode avant de jouer").group("Jeu").keys("Ctrl+G").icon(Icon::Gamepad),
            Command::new("clean_safe", "Nettoyer les fantômes sûrs").group("Chasseur").icon(Icon::Broom).enabled(!self.safe_ghosts().is_empty()),
            Command::new("trim_bg", "Mettre en veille les apps en arrière-plan").group("Mémoire").icon(Icon::Sleep),
            Command::new("view:procs", "Vue Processus").group("Vue").keys("Ctrl+1").icon(Icon::ListView),
            Command::new("view:hunter", "Vue Chasseur de RAM").group("Vue").keys("Ctrl+2").icon(Icon::Ghost),
            Command::new("view:perf", "Vue Performances").group("Vue").keys("Ctrl+3").icon(Icon::Chart),
            Command::new("view:startup", "Vue Démarrage").group("Vue").keys("Ctrl+4").icon(Icon::Power),
            Command::new("tree", "Arbre / liste à plat").group("Vue").keys("Ctrl+T").icon(Icon::Tree),
            Command::new("theme", if self.s.dark { "Thème clair" } else { "Thème sombre" }).group("Vue").icon(if self.s.dark { Icon::Sun } else { Icon::Moon }),
            Command::new("sample", "Mesurer maintenant").group("Système").keys("F5").icon(Icon::Refresh),
            Command::new("taskmgr", if self.taskmgr { "Rendre Ctrl+Maj+Échap au Gestionnaire Windows" } else { "Remplacer le Gestionnaire des tâches" }).group("Système").icon(Icon::Settings),
            Command::new("help", "Raccourcis clavier").group("Aide").keys("F1").icon(Icon::Keyboard),
        ];
        if self.game_active {
            v.insert(0, Command::new("game_exit", "Quitter le mode jeu").group("Jeu").icon(Icon::Gamepad));
        }
        if !self.elevated {
            v.push(Command::new("elevate", "Relancer en administrateur").group("Système").icon(Icon::Shield));
        }
        // V2 : projets en cours
        for pr in &self.projects {
            if pr.asleep() {
                v.push(Command::new(format!("wake:{}", pr.dir), format!("Réveiller le projet {}", pr.name)).group("Projets").icon(Icon::Bolt));
            } else {
                v.push(
                    Command::new(format!("sleep:{}", pr.dir), format!("Endormir le projet {} · {} processus, {}", pr.name, pr.keys.len(), human_bytes(pr.ram)))
                        .group("Projets")
                        .icon(Icon::Sleep),
                );
            }
        }
        // processus correspondant à la saisie : « Aller à » et « Terminer tous les … »
        let q = self.palette.query().trim().to_lowercase();
        // V1 : « :3000 » → terminer ce qui écoute sur ce port (Entrée, ou Suppr directement)
        if let (Some(port), Some(snap)) = (port_query(&q), &self.snap) {
            let owners: Vec<&Proc> = snap.procs.iter().filter(|x| x.ports.iter().any(|pp| pp.tcp && pp.port == port)).collect();
            if owners.is_empty() {
                v.insert(0, Command::new(format!("port:{port}"), format!("Rien n’écoute sur :{port}")).group("Port").icon(Icon::Info).enabled(false));
            } else {
                let who: Vec<String> = owners.iter().take(2).map(|x| format!("{} ({})", x.name, x.key.pid)).collect();
                let ram: u64 = owners.iter().map(|x| x.mem).sum();
                v.insert(
                    0,
                    Command::new(format!("port:{port}"), format!(":{port} — terminer {} · {}", who.join(", "), human_bytes(ram)))
                        .group("Port")
                        .keys("Suppr")
                        .icon(Icon::Close),
                );
            }
        }
        if let Some(snap) = &self.snap {
            if q.len() >= 2 && !q.starts_with(':') {
                let mut ps: Vec<&Proc> = snap.procs.iter().filter(|x| socle::fuzzy::score(&x.name, &q).is_some() || x.key.pid.to_string() == q).collect();
                ps.sort_by(|a, b| b.mem.cmp(&a.mem));
                for pr in ps.iter().take(6) {
                    v.push(
                        Command::new(format!("goto:{}:{}", pr.key.pid, pr.key.ctime), format!("Aller à {} ({}) · {}", pr.name, pr.key.pid, human_bytes(pr.mem)))
                            .group("Processus")
                            .icon(Icon::ArrowRight),
                    );
                }
                let mut names: Vec<&str> = ps.iter().map(|p| &*p.name).collect();
                names.sort();
                names.dedup();
                for n in names.into_iter().take(2) {
                    let count = snap.procs.iter().filter(|x| &*x.name == n).count();
                    v.push(Command::new(format!("killname:{n}"), format!("Terminer tous les « {n} » ({count})")).group("Processus").icon(Icon::Close));
                }
            }
        }
        v
    }

    fn run_command(&mut self, id: &str) {
        let a = match id {
            "game" => Action::GameMode,
            "game_exit" => Action::GameExit,
            "clean_safe" => Action::Clean(self.safe_ghosts().iter().map(|g| g.key).collect()),
            "trim_bg" => Action::Trim(self.trim_candidates()),
            "view:procs" => Action::SetView(View::Procs),
            "view:hunter" => Action::SetView(View::Hunter),
            "view:perf" => Action::SetView(View::Perf),
            "view:startup" => Action::SetView(View::Startup),
            "tree" => {
                self.s.tree = !self.s.tree;
                self.rows_dirty = true;
                return;
            }
            "theme" => {
                let d = !self.s.dark;
                self.set_dark(d);
                return;
            }
            "sample" => Action::SampleNow,
            "taskmgr" => Action::TaskmgrToggle,
            "help" => {
                self.help = true;
                return;
            }
            "elevate" => Action::Elevate,
            _ => {
                if let Some(port) = id.strip_prefix("port:").and_then(|x| x.parse().ok()) {
                    self.actions.push(Action::KillPort(port));
                } else if let Some(dir) = id.strip_prefix("sleep:") {
                    self.actions.push(Action::SleepProject(dir.to_string()));
                } else if let Some(dir) = id.strip_prefix("wake:") {
                    self.actions.push(Action::WakeProject(dir.to_string()));
                } else if let Some(rest) = id.strip_prefix("goto:") {
                    let mut it = rest.split(':');
                    if let (Some(pid), Some(ct)) = (it.next().and_then(|x| x.parse().ok()), it.next().and_then(|x| x.parse().ok())) {
                        self.actions.push(Action::Select(Key { pid, ctime: ct }));
                    }
                } else if let Some(n) = id.strip_prefix("killname:") {
                    let keys: Vec<Key> = self.snap.as_ref().map(|s| s.procs.iter().filter(|x| &*x.name == n).map(|x| x.key).collect()).unwrap_or_default();
                    self.actions.push(Action::Kill(keys, true));
                }
                return;
            }
        };
        self.actions.push(a);
    }

    /// Palette de commandes Ctrl+K du socle (recherche floue, récents en tête).
    fn draw_palette(&mut self, ui: &mut Ui) {
        if !self.palette.is_open() {
            return;
        }
        let ctx = ui.ctx().clone();
        // V1 : « :3000 » puis Suppr = terminer tout de suite (avec confirmation selon le réglage)
        if let Some(port) = port_query(self.palette.query()) {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
                self.palette.close();
                self.actions.push(Action::KillPort(port));
                return;
            }
        }
        let cmds = self.palette_commands();
        let p = self.pal;
        if let Some(id) = self.palette.show(&ctx, &p, &cmds) {
            self.run_command(&id);
        }
    }

    // ---------- Clavier ----------
    pub fn handle_keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers as M};
        // Ctrl+K (socle) ; Ctrl+P reste un alias
        if self.confirm.is_none() && !self.game_dialog {
            // (Maj testée avant : consume ignore Maj et avalerait Ctrl+Maj+P)
            let ctrl_p_sans_maj = ctx.input(|i| {
                i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::P, pressed: true, modifiers, .. } if modifiers.command && !modifiers.shift))
            });
            if !self.palette.handle_shortcut(ctx) && ctrl_p_sans_maj && socle::keys::consume(ctx, egui::Modifiers::COMMAND, Key::P) {
                self.palette.open();
            }
        }
        if self.palette.is_open() || self.confirm.is_some() || self.game_dialog {
            return;
        }
        // Ctrl+1…4 : touche physique (socle::keys), marche en AZERTY
        if let Some(n) = socle::keys::consume_ctrl_digit(ctx) {
            match n {
                1 => self.s.view = View::Procs,
                2 => self.s.view = View::Hunter,
                3 => self.s.view = View::Perf,
                4 => self.s.view = View::Startup,
                _ => {}
            }
        }
        let typing = ctx.memory(|m| m.focused().is_some()) && ctx.egui_wants_keyboard_input();
        let mut acts = Vec::new();
        ctx.input_mut(|i| {
            // Vues : Ctrl+Maj+lettre (pas de Ctrl+chiffre : en AZERTY, egui ne reçoit pas les chiffres).
            // Avant Ctrl+P / F / G : consume_key ignore Maj quand elle n'est pas précisée.
            for (k, v) in [(Key::P, View::Procs), (Key::F, View::Hunter), (Key::G, View::Perf), (Key::D, View::Startup)] {
                if i.consume_key(M::COMMAND | M::SHIFT, k) {
                    self.s.view = v;
                }
            }
            if i.consume_key(M::COMMAND, Key::F) {
                self.s.view = View::Procs;
                self.focus_search = true;
            }
            if i.consume_key(M::COMMAND, Key::G) {
                acts.push(Action::GameMode);
            }
            if i.consume_key(M::NONE, Key::F5) {
                acts.push(Action::SampleNow);
            }
            if i.consume_key(M::NONE, Key::F1) {
                self.help = !self.help;
            }
            if typing || self.s.view != View::Procs {
                return;
            }
            if i.consume_key(M::COMMAND, Key::T) {
                self.s.tree = !self.s.tree;
                self.rows_dirty = true;
            }
            let page = 15;
            for (k, d) in [(Key::ArrowDown, 1), (Key::ArrowUp, -1), (Key::PageDown, page), (Key::PageUp, -page), (Key::End, 100_000), (Key::Home, -100_000)] {
                if i.consume_key(M::NONE, k) {
                    self.move_cursor(d, false);
                }
                if i.consume_key(M::SHIFT, k) {
                    self.move_cursor(d, true);
                }
            }
            if self.s.tree {
                if let Some(c) = self.cursor {
                    if i.consume_key(M::NONE, Key::ArrowLeft) {
                        let row = self.rows.iter().find(|r| self.snap.as_ref().map(|s| s.procs[r.idx].key == c).unwrap_or(false));
                        if row.map(|r| r.open).unwrap_or(false) {
                            self.collapsed.insert(c);
                        } else if let Some(par) = self.proc(c).and_then(|p| p.parent) {
                            self.cursor = Some(par);
                            self.selected = [par].into_iter().collect();
                            self.scroll_to_cursor = true;
                        }
                        self.rows_dirty = true;
                    }
                    if i.consume_key(M::NONE, Key::ArrowRight) {
                        self.collapsed.remove(&c);
                        self.rows_dirty = true;
                    }
                }
            }
            if i.consume_key(M::COMMAND, Key::A) {
                self.selected = self.row_keys().into_iter().collect();
            }
            let t = self.targets();
            if !t.is_empty() {
                if i.consume_key(M::SHIFT, Key::Delete) {
                    acts.push(Action::Kill(t.clone(), true));
                }
                if i.consume_key(M::NONE, Key::Delete) {
                    acts.push(Action::Kill(t.clone(), false));
                }
                if i.consume_key(M::NONE, Key::Space) {
                    let susp = self.proc(t[0]).map(|p| p.suspended).unwrap_or(false);
                    acts.push(Action::Suspend(t.clone(), !susp));
                }
                if i.consume_key(M::NONE, Key::Enter) {
                    acts.push(Action::Focus(t[0]));
                }
                if i.consume_key(M::COMMAND, Key::E) {
                    acts.push(Action::OpenLocation(t[0]));
                }
                if i.consume_key(M::COMMAND | M::SHIFT, Key::C) {
                    if let Some(path) = self.proc(t[0]).and_then(|p| p.path.clone()) {
                        acts.push(Action::Copy(path.to_string()));
                    }
                } else if i.consume_key(M::COMMAND, Key::C) {
                    let s: Vec<String> = t.iter().filter_map(|k| self.proc(*k)).map(|p| format!("{} ({})", p.name, p.key.pid)).collect();
                    acts.push(Action::Copy(s.join("\n")));
                }
            }
            if i.consume_key(M::NONE, Key::Escape) {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.rows_dirty = true;
                } else {
                    self.selected.clear();
                }
            }
            // taper des lettres = rechercher
            let mut typed = String::new();
            i.events.retain(|e| match e {
                egui::Event::Text(t) if t.chars().all(|c| !c.is_control()) && !t.trim().is_empty() => {
                    typed.push_str(t);
                    false
                }
                _ => true,
            });
            if !typed.is_empty() {
                self.filter.push_str(&typed);
                self.focus_search = true;
                self.rows_dirty = true;
            }
        });
        self.actions.extend(acts);
    }
}

impl App {
    // ---------- V3 : jeux suivis ----------
    fn draw_games(&mut self, ui: &mut Ui) {
        let p = self.pal;
        if self.s.game_peaks.is_empty() {
            return;
        }
        let avail = self.snap.as_ref().map(|s| s.totals.mem_total - s.totals.mem_used).unwrap_or(0);
        let recup: u64 = self.ghosts.iter().map(|g| g.ram).sum();
        section(ui, &p, "Jeux");
        let mut games: Vec<crate::qol::GamePeak> = self.s.game_peaks.values().cloned().collect();
        games.sort_by(|a, b| b.seen.cmp(&a.seen));
        for g in games.iter().take(8) {
            let miss = crate::qol::missing(g.peak, avail);
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icons::paint(ui.painter(), ir, Icon::Gamepad, p.muted);
                ui.label(RichText::new(&g.name).size(13.0));
                ui.label(RichText::new(format!("pic {}", human_bytes(g.peak))).size(12.0).color(p.muted));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if miss == 0 {
                        ui.label(RichText::new("ça passe").size(12.0).color(p.add));
                    } else if miss <= recup {
                        ui.label(RichText::new(format!("il te manque {} — le chasseur en récupère {}", human_bytes(miss), human_bytes(recup))).size(12.0).color(p.live));
                    } else {
                        ui.label(RichText::new(format!("il te manque {}", human_bytes(miss))).size(12.0).color(p.del));
                    }
                });
            });
        }
        ui.add_space(10.0);
    }

    // ---------- Journal ----------
    fn draw_journal(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let now = crate::journal::now();
        let week: Vec<&crate::journal::Rec> = self.journal.iter().filter(|r| now.saturating_sub(r.t) < 7 * 86_400).collect();
        section(ui, &p, "Journal");
        if week.is_empty() {
            ui.label(RichText::new("Rien de nettoyé ces 7 derniers jours.").color(p.muted).size(12.0));
            return;
        }
        let total: u64 = week.iter().map(|r| r.bytes).sum();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} récupérés cette semaine", human_bytes(total))).size(15.0).strong().color(p.add));
            ui.label(RichText::new(format!("· {} action(s)", week.len())).color(p.muted));
        });
        // coupables récurrents
        let mut by: std::collections::HashMap<&str, (u64, usize)> = std::collections::HashMap::new();
        for r in &week {
            let e = by.entry(r.name.as_str()).or_default();
            e.0 += r.bytes;
            e.1 += 1;
        }
        let mut top: Vec<(&str, (u64, usize))> = by.into_iter().collect();
        top.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        if let Some((n, (b, c))) = top.first() {
            ui.label(RichText::new(format!("Coupable n°1 : {n} — {} en {c} fois", human_bytes(*b))).color(p.live).size(12.5));
        }
        egui::CollapsingHeader::new(RichText::new("Détail").color(p.muted).size(12.0)).id_salt("journal").show(ui, |ui| {
            egui::Grid::new("jr").num_columns(4).spacing(vec2(16.0, 3.0)).show(ui, |ui| {
                for r in self.journal.iter().rev().take(25) {
                    ui.label(RichText::new(crate::journal::ago(r.t)).color(p.muted).size(11.5));
                    ui.label(RichText::new(&r.what).size(11.5));
                    ui.label(RichText::new(&r.name).size(11.5));
                    ui.label(RichText::new(human_bytes(r.bytes)).color(p.add).size(11.5));
                    ui.end_row();
                }
            });
        });
    }

    // ---------- Démarrage ----------
    fn draw_startup(&mut self, ui: &mut Ui) {
        let p = self.pal;
        if self.startup.is_none() {
            self.startup = Some(crate::startup::list());
        }
        let list = self.startup.clone().unwrap_or_default();
        // RAM actuelle de chaque app (processus lancés depuis le même exe)
        // table calculée à la réception de la mesure (pas à chaque image)
        if self.startup_ram.is_empty() {
            if let Some(snap) = &self.snap {
                self.startup_ram = crate::startup::ram_table(&list, &snap.procs);
            }
        }
        let ram = self.startup_ram.clone();
        let mut acts = Vec::new();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            Frame::new().inner_margin(Margin::symmetric(24, 18)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let on = list.iter().filter(|e| e.enabled).count();
                let running: u64 = list.iter().filter(|e| e.enabled).filter_map(|e| e.exe.as_ref().and_then(|x| ram.get(x))).map(|r| r.0).sum();
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(format!("{on} app(s) au démarrage sur {}", list.len())).size(22.0).strong());
                        ui.label(
                            RichText::new(format!("Celles qui tournent occupent {} en ce moment. Désactiver = ne plus se lancer avec Windows (rien n’est désinstallé).", human_bytes(running)))
                                .color(p.muted),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icons::wide(ui, Icon::Refresh, "Actualiser", "", 130.0, None, &p).clicked() {
                            acts.push(Action::StartupRefresh);
                        }
                    });
                });
                ui.add_space(12.0);
                for (i, e) in list.iter().enumerate() {
                    let r_now = e.exe.as_ref().and_then(|x| ram.get(x)).copied();
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::hover());
                    let bg = if resp.hovered() { p.hover_bg } else { Color32::TRANSPARENT };
                    ui.painter().rect_filled(rect, CornerRadius::same(8), bg);
                    // interrupteur
                    let sw = Rect::from_center_size(pos2(rect.left() + 28.0, rect.center().y), vec2(36.0, 20.0));
                    let sr = ui.interact(sw, ui.id().with(("sw", i)), Sense::click());
                    ui.painter().rect_filled(sw, CornerRadius::same(10), if e.enabled { p.accent } else { p.soft });
                    let kx = if e.enabled { sw.right() - 10.0 } else { sw.left() + 10.0 };
                    ui.painter().circle_filled(pos2(kx, sw.center().y), 7.0, if e.enabled { p.on_accent } else { p.muted });
                    let locked = e.source.needs_admin() && !self.elevated;
                    let sr = sr.on_hover_text(if locked { "Entrée « tous les utilisateurs » : relance Vigie en admin pour la modifier" } else if e.enabled { "Désactiver au démarrage" } else { "Activer au démarrage" });
                    if sr.clicked() {
                        if locked {
                            acts.push(Action::Elevate);
                        } else {
                            acts.push(Action::StartupToggle(i, !e.enabled));
                        }
                    }
                    let mut x = rect.left() + 58.0;
                    if let Some(exe) = e.exe.as_ref().filter(|x| std::path::Path::new(x.as_str()).is_file()) {
                        let key: Arc<str> = Arc::from(exe.as_str());
                        if let Some(tex) = self.icon(&key).and_then(|en| en.tex.clone()) {
                            ui.painter().image(tex.id(), Rect::from_center_size(pos2(x + 9.0, rect.center().y), vec2(18.0, 18.0)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                    }
                    x += 28.0;
                    let maxw = rect.right() - 260.0 - x;
                    let top = Rect::from_min_max(pos2(x, rect.top() + 4.0), pos2(rect.right(), rect.center().y + 2.0));
                    let bot = Rect::from_min_max(pos2(x, rect.center().y), pos2(rect.right(), rect.bottom() - 4.0));
                    text_left(ui, top, x, &e.name, FontId::proportional(14.0), if e.enabled { p.text } else { p.muted }, maxw);
                    text_left(ui, bot, x, &format!("{} · {}", e.source.label(), e.command), FontId::proportional(11.5), p.muted, maxw);
                    match r_now {
                        Some((b, n)) => {
                            ui.painter().text(pos2(rect.right() - 120.0, rect.center().y), Align2::RIGHT_CENTER, human_bytes(b), FontId::proportional(14.0), p.live);
                            ui.painter().text(pos2(rect.right() - 16.0, rect.center().y), Align2::RIGHT_CENTER, format!("en cours · {n}"), FontId::proportional(11.5), p.muted);
                        }
                        None => {
                            ui.painter().text(pos2(rect.right() - 16.0, rect.center().y), Align2::RIGHT_CENTER, "ne tourne pas", FontId::proportional(11.5), p.muted);
                        }
                    }
                }
                ui.add_space(10.0);
                ui.label(RichText::new("Sources : clés Run du registre (utilisateur, machine, 32 bits) et dossiers Démarrage — les mêmes que le Gestionnaire des tâches. Les tâches planifiées et services ne sont pas listés.").size(11.5).color(p.muted));
            });
        });
        self.actions.extend(acts);
    }
}

/// « :3000 » ou « 3000 » (que des chiffres) → port.
fn port_query(q: &str) -> Option<u16> {
    let t = q.trim();
    let d = t.strip_prefix(':')?;
    (!d.is_empty() && d.chars().all(|c| c.is_ascii_digit())).then(|| d.parse().ok()).flatten()
}

/// Index de l'anneau de la boîte noire sous l'abscisse `x` (graphiques alignés à droite, comme `graph`).
fn bb_index(rect: Rect, x: f32, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let step = rect.width() / (crate::app::HIST - 1) as f32;
    let from_right = ((rect.right() - x) / step).round().max(0.0) as usize;
    (from_right < len).then(|| len - 1 - from_right)
}

fn bb_x(rect: Rect, i: usize, len: usize) -> f32 {
    let step = rect.width() / (crate::app::HIST - 1) as f32;
    rect.right() - (len - 1 - i) as f32 * step
}

fn fmt_metric(m: crate::qol::Metric, v: f64) -> String {
    use crate::qol::Metric;
    match m {
        Metric::Cpu | Metric::Gpu => pct(v as f32),
        Metric::Mem => human_bytes(v as u64),
        Metric::Disk => human_rate(v),
    }
}

fn fmt_int(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{202F}');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::app::{App, Settings, View};
    use egui::{Event, Key, Modifiers, RawInput};

    fn press(ctx: &egui::Context, app: &mut App, key: Key, m: Modifiers) {
        let mut raw = RawInput::default();
        raw.events.push(Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: m });
        let _ = ctx.run_ui(raw, |ui| app.handle_keys(ui.ctx()));
    }

    #[test]
    fn vues_ctrl_maj_lettre() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, Settings::default());
        let cs = Modifiers { ctrl: true, shift: true, command: true, ..Default::default() };
        for (k, v) in [(Key::F, View::Hunter), (Key::G, View::Perf), (Key::D, View::Startup), (Key::P, View::Procs)] {
            press(&ctx, &mut app, k, cs);
            assert_eq!(app.s.view, v, "Ctrl+Maj+{k:?}");
        }
        assert!(!app.palette.is_open(), "Ctrl+Maj+P ne doit pas ouvrir la palette");
        // Ctrl+P seul : palette, pas de changement de vue
        app.s.view = View::Hunter;
        press(&ctx, &mut app, Key::P, Modifiers { ctrl: true, command: true, ..Default::default() });
        assert!(app.palette.is_open());
        assert_eq!(app.s.view, View::Hunter);
    }

    #[test]
    fn port_query() {
        assert_eq!(super::port_query(":3000"), Some(3000));
        assert_eq!(super::port_query(" :5173 "), Some(5173));
        assert_eq!(super::port_query("3000"), None);
        assert_eq!(super::port_query(":abc"), None);
        assert_eq!(super::port_query(":"), None);
    }

    /// AZERTY : la touche 4 envoie « ' » (Key::Quote) avec la touche physique Num4.
    #[test]
    fn ctrl_chiffre_azerty() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, Settings::default());
        let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
        let mut raw = RawInput::default();
        raw.events.push(Event::Key { key: Key::Quote, physical_key: Some(Key::Num4), pressed: true, repeat: false, modifiers: ctrl });
        let _ = ctx.run_ui(raw, |ui| app.handle_keys(ui.ctx()));
        assert_eq!(app.s.view, View::Startup);
        press(&ctx, &mut app, Key::Num2, ctrl);
        assert_eq!(app.s.view, View::Hunter);
    }
}
