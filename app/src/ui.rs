use crate::worker::{Event, Worker};
use chrono::{DateTime, Local};
use eframe::egui::{self, Color32, FontId, RichText, Stroke, Vec2};
use misf_core::model::{CheckResult, Host};
use serde_json::{Value, json};
use zeroize::Zeroize;

const NAVY: Color32 = Color32::from_rgb(19, 36, 59);
const BG: Color32 = Color32::from_rgb(244, 247, 251);
const INK: Color32 = Color32::from_rgb(34, 53, 77);
const MUTED: Color32 = Color32::from_rgb(109, 127, 150);
const BLUE: Color32 = Color32::from_rgb(33, 117, 195);
const BORDER: Color32 = Color32::from_rgb(220, 229, 239);

struct Editor {
    host: Host,
    port: String,
    show_secrets: bool,
    error: String,
}
struct TrustDialog {
    host_id: String,
    address: String,
    old: String,
    actual: String,
    next: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum VaultMode {
    Create,
    Unlock,
    Manage,
    AddKey,
}
struct VaultDialog {
    mode: VaultMode,
    password: String,
    confirmation: String,
    error: String,
}
impl Drop for VaultDialog {
    fn drop(&mut self) {
        self.password.zeroize();
        self.confirmation.zeroize();
    }
}

pub struct ManagementApp {
    worker: Worker,
    hosts: Vec<Host>,
    selected: String,
    search: String,
    package_search: String,
    path: String,
    busy: bool,
    status: String,
    error: Option<String>,
    editor: Option<Editor>,
    delete_id: Option<String>,
    trust: Option<TrustDialog>,
    after_trust: Option<String>,
    tab: usize,
    encrypted: bool,
    locked: bool,
    plaintext_exists: bool,
    slots: Vec<usize>,
    storage_warning: String,
    vault_dialog: Option<VaultDialog>,
    #[cfg(feature = "capture")]
    capture_started: std::time::Instant,
    #[cfg(feature = "capture")]
    capture_requested: bool,
}

impl ManagementApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style_of(egui::Theme::Light)).clone();
        style.visuals = egui::Visuals::light();
        style.visuals.override_text_color = Some(INK);
        style.visuals.panel_fill = BG;
        style.visuals.selection.bg_fill = Color32::from_rgb(218, 235, 251);
        style.spacing.item_spacing = Vec2::new(10.0, 10.0);
        style.spacing.button_padding = Vec2::new(14.0, 9.0);
        style.spacing.interact_size.y = 34.0;
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(14.0));
        cc.egui_ctx.set_theme(egui::ThemePreference::Light);
        cc.egui_ctx.set_style_of(egui::Theme::Light, style);
        let worker = Worker::new(cc.egui_ctx.clone());
        let mut app = Self {
            worker,
            hosts: vec![],
            selected: String::new(),
            search: String::new(),
            package_search: String::new(),
            path: String::new(),
            busy: false,
            status: String::new(),
            error: None,
            editor: None,
            delete_id: None,
            trust: None,
            after_trust: None,
            tab: 0,
            encrypted: false,
            locked: false,
            plaintext_exists: false,
            slots: vec![],
            storage_warning: String::new(),
            vault_dialog: None,
            #[cfg(feature = "capture")]
            capture_started: std::time::Instant::now(),
            #[cfg(feature = "capture")]
            capture_requested: false,
        };
        app.send(json!({"action":"storage_status"}));
        #[cfg(feature = "capture")]
        if std::env::var_os("MISF_OPEN_EDITOR").is_some() {
            app.open_editor(None);
        }
        #[cfg(feature = "capture")]
        if std::env::var_os("MISF_OPEN_VAULT_SETUP").is_some() {
            app.open_vault_dialog(VaultMode::Create);
        }
        app
    }

    fn current(&self) -> Option<&Host> {
        self.hosts.iter().find(|host| host.id == self.selected)
    }
    fn send(&mut self, request: Value) {
        self.busy = true;
        self.status = "處理中…".into();
        if self.worker.requests.send(request).is_err() {
            self.busy = false;
            self.error = Some("本機工作執行緒已結束，請重啟程式".into());
        }
    }

    fn process_events(&mut self) {
        while let Ok(event) = self.worker.events.try_recv() {
            match event {
                Event::Progress(message) => self.status = message,
                Event::Done { action, result } => {
                    self.busy = false;
                    match result {
                        Ok(data) => self.received(&action, data),
                        Err(error) => {
                            self.status = "操作失敗".into();
                            self.error = Some(error);
                            if action == "vault_create" {
                                self.vault_dialog = None;
                                self.send(json!({"action":"storage_status"}));
                            }
                        }
                    }
                }
            }
        }
    }

    fn received(&mut self, action: &str, data: Value) {
        match action {
            "storage_status" => {
                self.apply_storage(&data);
                if self.locked {
                    self.clear_host_cache();
                    self.open_vault_dialog(VaultMode::Unlock);
                    self.status = "主機庫已鎖定".into();
                } else {
                    self.send(json!({"action":"list"}));
                }
            }
            "vault_create" | "vault_unlock" => {
                self.apply_storage(&data);
                self.vault_dialog = None;
                self.send(json!({"action":"list"}));
            }
            "vault_lock" => {
                self.apply_storage(&data);
                self.clear_host_cache();
                self.vault_dialog = None;
                self.status = "主機庫已鎖定，解鎖金鑰已釋放".into();
            }
            "vault_add_key" => {
                self.apply_storage(&data["storage"]);
                self.open_vault_dialog(VaultMode::Manage);
                self.status = format!("已新增解鎖金鑰槽 #{}", data["slot"]);
            }
            "vault_backup" => {
                self.status = format!(
                    "已匯出加密備份：{}",
                    data["path"].as_str().unwrap_or_default()
                )
            }
            "list" => {
                match serde_json::from_value::<Vec<Host>>(data["hosts"].clone()) {
                    Ok(hosts) => self.hosts = hosts,
                    Err(error) => {
                        self.error = Some(format!("無法讀取主機資料：{error}"));
                        return;
                    }
                }
                self.path = data["path"].as_str().unwrap_or_default().into();
                self.encrypted = data["encrypted"].as_bool().unwrap_or(false);
                if !self.hosts.iter().any(|h| h.id == self.selected) {
                    self.selected = self.hosts.first().map(|h| h.id.clone()).unwrap_or_default();
                }
                self.status = format!("已載入 {} 台主機", self.hosts.len());
            }
            "save" => {
                self.selected = data["id"].as_str().unwrap_or_default().into();
                self.editor = None;
                self.send(json!({"action":"list"}));
            }
            "delete" => {
                self.delete_id = None;
                self.selected.clear();
                self.send(json!({"action":"list"}));
            }
            "probe" => {
                if let Some(host) = self.current() {
                    let actual = data["fingerprint"].as_str().unwrap_or_default().to_owned();
                    if host.fingerprint == actual {
                        self.status = "SSH 主機指紋一致".into();
                        if let Some(action) = self.after_trust.take() {
                            self.send(json!({"action":action,"id":self.selected}));
                        }
                    } else {
                        self.trust = Some(TrustDialog {
                            host_id: host.id.clone(),
                            address: format!("{}:{}", host.address, host.port),
                            old: host.fingerprint.clone(),
                            actual,
                            next: self.after_trust.take(),
                        });
                        self.status = "請核對 SSH 主機指紋".into();
                    }
                }
            }
            "trust" => {
                if let Some(trust) = self.trust.take() {
                    if let Some(host) = self.hosts.iter_mut().find(|h| h.id == trust.host_id) {
                        host.fingerprint = trust.actual;
                    }
                    self.status = "已儲存 SSH 主機指紋".into();
                    if let Some(action) = trust.next {
                        self.send(json!({"action":action,"id":self.selected}));
                    }
                }
            }
            "test" => {
                self.status = format!(
                    "SSH 連線成功 · 遠端系統：{}",
                    data["os"].as_str().unwrap_or_default()
                )
            }
            "check" => match serde_json::from_value::<CheckResult>(data) {
                Ok(result) => {
                    self.status = if result.status == "success" {
                        "APT 檢查完成，未安裝任何套件"
                    } else {
                        "APT 檢查失敗，請查看錯誤與執行輸出"
                    }
                    .into();
                    if let Some(host) = self.hosts.iter_mut().find(|h| h.id == self.selected) {
                        host.last_check = Some(result);
                    }
                }
                Err(error) => self.error = Some(error.to_string()),
            },
            _ => {}
        }
    }

    fn start_connection_action(&mut self, action: &str) {
        if let Some(host) = self.current() {
            if host.fingerprint.is_empty() {
                self.after_trust = Some(action.into());
                self.send(json!({"action":"probe","id":self.selected}));
            } else {
                self.send(json!({"action":action,"id":self.selected}));
            }
        }
    }

    fn open_editor(&mut self, host: Option<Host>) {
        if self.locked {
            return;
        }
        let host = host.unwrap_or_else(|| Host {
            port: 22,
            os: "Linux".into(),
            auth_type: "password".into(),
            use_pty: true,
            ..Default::default()
        });
        self.editor = Some(Editor {
            port: host.port.to_string(),
            host,
            show_secrets: false,
            error: String::new(),
        });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.visuals_mut().override_text_color = Some(Color32::from_rgb(220, 231, 245));
        ui.add_space(9.0);
        ui.label(
            RichText::new("MISF")
                .size(34.0)
                .strong()
                .color(Color32::WHITE),
        );
        ui.label(
            RichText::new("MANAGEMENT TERMINAL")
                .size(10.0)
                .color(Color32::from_rgb(143, 166, 197)),
        );
        ui.add_space(23.0);
        ui.scope(|ui| {
            ui.visuals_mut().override_text_color = Some(INK);
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .text_color(INK)
                    .hint_text(RichText::new("搜尋主機、位址或備註").color(MUTED))
                    .desired_width(f32::INFINITY),
            );
        });
        ui.add_space(10.0);
        ui.label(
            RichText::new(format!("主機清單  /  {}", self.hosts.len()))
                .size(12.0)
                .color(Color32::from_rgb(147, 174, 208)),
        );
        ui.add_space(6.0);
        let query = self.search.to_lowercase();
        let visible: Vec<_> = self
            .hosts
            .iter()
            .filter(|h| {
                format!("{} {} {}", h.name, h.address, h.notes)
                    .to_lowercase()
                    .contains(&query)
            })
            .cloned()
            .collect();
        egui::ScrollArea::vertical()
            .max_height((ui.available_height() - 98.0).max(80.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if visible.is_empty() {
                    ui.add_space(30.0);
                    ui.label(
                        RichText::new(if self.hosts.is_empty() {
                            "尚未新增主機"
                        } else {
                            "沒有符合的主機"
                        })
                        .color(Color32::from_rgb(143, 166, 197)),
                    );
                }
                for host in visible {
                    let selected = self.selected == host.id;
                    let fill = if selected {
                        Color32::from_rgb(40, 66, 95)
                    } else {
                        NAVY
                    };
                    let card = egui::Frame::new()
                        .fill(fill)
                        .corner_radius(7)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width() - 2.0);
                            let success = host
                                .last_check
                                .as_ref()
                                .is_some_and(|r| r.status == "success");
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("●").size(10.0).color(if success {
                                    Color32::from_rgb(78, 201, 162)
                                } else {
                                    MUTED
                                }));
                                ui.label(
                                    RichText::new(&host.name)
                                        .strong()
                                        .size(15.0)
                                        .color(Color32::WHITE),
                                );
                            });
                            ui.label(
                                RichText::new(format!(
                                    "{}@{}:{}",
                                    host.username, host.address, host.port
                                ))
                                .size(11.0)
                                .color(Color32::from_rgb(166, 187, 213)),
                            );
                        })
                        .response;
                    if ui
                        .interact(card.rect, card.id, egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.selected = host.id;
                    }
                    ui.add_space(5.0);
                }
            });
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.label(
                RichText::new("本機模式 · 全 Rust · 內建 SSH")
                    .size(11.0)
                    .color(Color32::from_rgb(143, 166, 197)),
            );
            if ui
                .add_enabled(
                    !self.locked,
                    primary("＋ 新增主機").min_size([ui.available_width(), 38.0].into()),
                )
                .clicked()
            {
                self.open_editor(None);
            }
        });
    }

    fn workspace(&mut self, ui: &mut egui::Ui) {
        if self.locked {
            ui.label(
                RichText::new("LUKSBOX VAULT")
                    .size(10.0)
                    .strong()
                    .color(MUTED),
            );
            ui.add_space(12.0);
            ui.heading(RichText::new("主機庫已鎖定").size(29.0));
            ui.label("輸入任一已註冊的解鎖密碼後，才能讀取主機資料與憑證。");
            ui.add_space(24.0);
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.add_space(15.0);
                ui.label("LUKSbox · AES-256-GCM-SIV · 多金鑰解鎖");
                ui.add_space(12.0);
                if ui.add(primary("解鎖主機庫")).clicked() {
                    self.open_vault_dialog(VaultMode::Unlock);
                }
                ui.add_space(15.0);
            });
            self.storage_notice(ui);
            return;
        }
        let host = self.current().cloned();
        ui.label(
            RichText::new("HOST OVERVIEW")
                .size(10.0)
                .strong()
                .color(MUTED),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(
                    host.as_ref()
                        .map(|h| h.name.as_str())
                        .unwrap_or("新增第一台主機"),
                )
                .size(29.0)
                .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        host.is_some(),
                        egui::Button::new(
                            RichText::new("刪除").color(Color32::from_rgb(174, 65, 65)),
                        ),
                    )
                    .clicked()
                {
                    self.delete_id = Some(self.selected.clone());
                }
                if ui
                    .add_enabled(host.is_some(), egui::Button::new("編輯主機"))
                    .clicked()
                {
                    self.open_editor(host.clone());
                }
            });
        });
        let Some(host) = host else {
            ui.label(RichText::new("新增 Linux／BSD 主機，使用 SSH Key 或帳密連線。").color(MUTED));
            ui.add_space(50.0);
            card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.add_space(24.0);
                ui.label(RichText::new("開始管理你的主機").size(23.0).strong());
                ui.add_space(9.0);
                ui.label("新增連線資料、確認主機指紋，再一鍵列出 APT 待更新套件。");
                ui.add_space(18.0);
                if ui.add(primary("＋ 新增第一台主機")).clicked() {
                    self.open_editor(None);
                }
                ui.add_space(24.0);
            });
            self.storage_notice(ui);
            return;
        };
        ui.label(
            RichText::new(format!(
                "{}@{}:{}  ·  {}  ·  {}",
                host.username,
                host.address,
                host.port,
                host.os,
                if host.auth_type == "key" {
                    "SSH Key"
                } else {
                    "密碼登入"
                }
            ))
            .color(MUTED),
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("測試連線").clicked() {
                self.start_connection_action("test");
            }
            if ui.button("主機指紋").clicked() {
                self.after_trust = None;
                self.send(json!({"action":"probe","id":self.selected}));
            }
            if ui
                .add_enabled(host.os != "BSD", primary("檢查 APT 更新"))
                .clicked()
            {
                self.start_connection_action("check");
            }
            if ui.button("重新載入").clicked() {
                self.send(json!({"action":"list"}));
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(if host.fingerprint.is_empty() {
                "首次連線前需確認 SSH 主機指紋".into()
            } else {
                format!("SSH 指紋  {}", host.fingerprint)
            })
            .size(11.0)
            .color(MUTED),
        );
        ui.add_space(12.0);
        let result = host.last_check.as_ref();
        let successful = result.is_some_and(|r| r.status == "success");
        ui.columns(3, |columns| {
            stat(
                &mut columns[0],
                "待更新套件",
                &if successful {
                    result.unwrap().packages.len().to_string()
                } else {
                    "—".into()
                },
            );
            stat(
                &mut columns[1],
                "檢查狀態",
                result
                    .map(|r| match r.status.as_str() {
                        "success" => "檢查完成",
                        "unsupported" => "不支援 APT",
                        _ => "檢查失敗",
                    })
                    .unwrap_or(if host.os == "BSD" {
                        "不支援 APT"
                    } else {
                        "尚未檢查"
                    }),
            );
            stat(
                &mut columns[2],
                "上次檢查",
                &result
                    .map(|r| format_time(&r.checked_at))
                    .unwrap_or("—".into()),
            );
        });
        ui.add_space(16.0);
        card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, 0, "套件清單");
                ui.selectable_value(&mut self.tab, 1, "執行輸出");
            });
            ui.separator();
            let available = (ui.available_height() - 145.0).max(120.0);
            if self.tab == 1 {
                egui::ScrollArea::both()
                    .max_height(available)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let text = result
                            .map(|r| format!("{}\n{}", r.output, r.error))
                            .unwrap_or("尚未執行檢查".into());
                        ui.label(RichText::new(text).monospace().size(12.0));
                    });
            } else {
                let message = match result {
                    Some(r) if r.status == "success" => {
                        if r.packages.is_empty() {
                            "套件索引已更新，目前沒有待更新套件。"
                        } else {
                            "以下套件可更新。此程式只列出，不會安裝或升級。"
                        }
                    }
                    Some(r) => &r.error,
                    None if host.os == "BSD" => "BSD 目前支援 SSH 連線測試；不支援 APT 套件檢查。",
                    None => "按「檢查 APT 更新」更新索引並取得套件清單。",
                };
                ui.label(RichText::new(message).color(
                    if result.is_some_and(|r| r.status == "failed") {
                        Color32::from_rgb(170, 65, 65)
                    } else {
                        MUTED
                    },
                ));
                ui.add(
                    egui::TextEdit::singleline(&mut self.package_search)
                        .hint_text("篩選套件名稱")
                        .desired_width(270.0),
                );
                ui.add_space(5.0);
                egui::ScrollArea::both()
                    .max_height((available - 75.0).max(100.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("packages")
                            .striped(true)
                            .min_col_width(130.0)
                            .spacing([20.0, 14.0])
                            .show(ui, |ui| {
                                for caption in ["套件", "目前版本", "可更新版本", "架構"]
                                {
                                    ui.label(RichText::new(caption).strong().color(MUTED));
                                }
                                ui.end_row();
                                if let Some(result) = result {
                                    for package in result.packages.iter().filter(|p| {
                                        p.name
                                            .to_lowercase()
                                            .contains(&self.package_search.to_lowercase())
                                    }) {
                                        ui.label(&package.name);
                                        ui.label(&package.current_version);
                                        ui.label(&package.candidate_version);
                                        ui.label(&package.architecture);
                                        ui.end_row();
                                    }
                                }
                            });
                        if result.is_none_or(|r| r.packages.is_empty()) {
                            ui.add_space(28.0);
                            ui.label(RichText::new("尚無套件資料").color(MUTED));
                        }
                    });
            }
        });
        self.storage_notice(ui);
    }

    fn storage_notice(&mut self, ui: &mut egui::Ui) {
        ui.add_space(15.0);
        ui.label(
            RichText::new(if self.encrypted {
                "LUKSbox 加密儲存 · AES-256-GCM-SIV · 實驗性功能"
            } else {
                "目前未啟用加密 · SSH 帳密與私鑰以明文儲存在此電腦"
            })
            .size(11.0)
            .color(Color32::from_rgb(151, 115, 56)),
        );
        ui.label(RichText::new(&self.path).size(10.0).color(MUTED));
        if !self.storage_warning.is_empty() {
            ui.colored_label(Color32::from_rgb(174, 65, 65), &self.storage_warning);
        }
        if self.encrypted && self.plaintext_exists {
            ui.colored_label(
                Color32::from_rgb(174, 105, 40),
                "原 hosts.json 明文副本仍存在，請確認後自行處理。",
            );
        }
        ui.horizontal_wrapped(|ui| {
            if self.encrypted {
                if self.locked {
                    if ui.button("解鎖主機庫").clicked() {
                        self.open_vault_dialog(VaultMode::Unlock);
                    }
                } else {
                    if ui.button("鎖定").clicked() {
                        self.send(json!({"action":"vault_lock"}));
                    }
                    if ui.button("管理解鎖金鑰").clicked() {
                        self.open_vault_dialog(VaultMode::Manage);
                    }
                    if ui.button("匯出加密備份…").clicked() {
                        self.choose_backup();
                    }
                }
            } else if ui.button("啟用 LUKSbox 加密…").clicked() {
                self.open_vault_dialog(VaultMode::Create);
            }
        });
    }

    fn apply_storage(&mut self, data: &Value) {
        self.encrypted = data["encrypted"].as_bool().unwrap_or(false);
        self.locked = data["locked"].as_bool().unwrap_or(false);
        self.plaintext_exists = data["plaintext_exists"].as_bool().unwrap_or(false);
        self.path = data["path"].as_str().unwrap_or_default().into();
        self.storage_warning = data["warning"].as_str().unwrap_or_default().into();
        self.slots = data["slots"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|slot| slot["index"].as_u64().map(|index| index as usize))
            .collect();
    }

    fn clear_host_cache(&mut self) {
        for host in &mut self.hosts {
            wipe_host(host);
        }
        self.hosts.clear();
        self.selected.clear();
        self.search.zeroize();
        self.package_search.zeroize();
        if let Some(editor) = &mut self.editor {
            wipe_host(&mut editor.host);
        }
        self.editor = None;
        self.trust = None;
        self.delete_id = None;
        self.after_trust = None;
    }

    fn open_vault_dialog(&mut self, mode: VaultMode) {
        self.vault_dialog = Some(VaultDialog {
            mode,
            password: String::new(),
            confirmation: String::new(),
            error: String::new(),
        });
    }

    fn choose_backup(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("匯出加密備份（使用新的檔名）")
            .set_file_name("misf-backup.lbx")
            .add_filter("LUKSbox", &["lbx"])
            .save_file()
        {
            self.send(json!({"action":"vault_backup","path":path}));
        }
    }

    fn vault_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.vault_dialog.take() else {
            return;
        };
        let mut submit = false;
        let mut cancel = false;
        let mut add = false;
        let mut backup = false;
        egui::Modal::new(egui::Id::unique("vault_dialog")).show(ctx, |ui| {
            ui.set_width(570.0);
            ui.heading(match dialog.mode { VaultMode::Create => "啟用 LUKSbox 加密", VaultMode::Unlock => "解鎖主機庫", VaultMode::Manage => "管理解鎖金鑰", VaultMode::AddKey => "新增解鎖密碼" });
            ui.add_space(12.0);
            ui.add_enabled_ui(!self.busy, |ui| {
                if dialog.mode == VaultMode::Manage {
                    ui.label(format!("已使用 {} / 8 個金鑰槽。任一密碼都能獨立解鎖相同主機庫。", self.slots.len()));
                    for slot in &self.slots { ui.label(format!("金鑰槽 #{slot} · 密碼")); }
                    ui.label("請保留至少一組可取回的備援密碼。此版先提供新增金鑰，不提供撤銷或硬體金鑰註冊。");
                    ui.horizontal(|ui| { add = ui.add_enabled(self.slots.len() < 8, primary("新增解鎖密碼")).clicked(); backup = ui.button("匯出加密備份…").clicked(); cancel = ui.button("關閉").clicked(); });
                } else {
                    if dialog.mode == VaultMode::Create {
                        ui.label("把目前主機資料轉成 LUKSbox .lbx。加密內容與中繼資料，使用密碼解鎖；不需要磁碟掛載驅動。");
                        ui.colored_label(Color32::from_rgb(174, 105, 40), "實驗性功能：LUKSbox 尚未完成付費第三方安全稽核。遺失所有解鎖密碼將無法復原。");
                        ui.label("轉換後保留原 hosts.json 作為明文副本；新日常讀寫使用 .lbx。請在驗證備份後自行處理明文副本與舊版 .msif-management 資料。");
                    }
                    field(ui, "解鎖密碼", &mut dialog.password, if dialog.mode == VaultMode::Unlock { "輸入任一已註冊密碼" } else { "至少 12 個字元，建議使用長密碼句" }, true);
                    if dialog.mode != VaultMode::Unlock { field(ui, "再次輸入密碼", &mut dialog.confirmation, "確認解鎖密碼", true); }
                    if !dialog.error.is_empty() { ui.colored_label(Color32::from_rgb(174, 65, 65), &dialog.error); }
                    ui.horizontal(|ui| { submit = ui.add(primary(match dialog.mode { VaultMode::Create => "加密並驗證", VaultMode::Unlock => "解鎖", _ => "新增金鑰" })).clicked(); cancel = ui.button("取消").clicked(); });
                }
            });
            if self.busy { ui.horizontal(|ui| { ui.spinner(); ui.label("正在處理加密資料，請稍候…"); }); }
        });
        if add {
            dialog.mode = VaultMode::AddKey;
        }
        if backup {
            self.choose_backup();
        }
        if submit {
            if dialog.mode != VaultMode::Unlock && dialog.password != dialog.confirmation {
                dialog.error = "兩次輸入的密碼不一致".into();
            } else if dialog.mode != VaultMode::Unlock && dialog.password.chars().count() < 12 {
                dialog.error = "解鎖密碼至少需要 12 個字元".into();
            } else if dialog.password.is_empty() {
                dialog.error = "請輸入解鎖密碼".into();
            } else {
                dialog.error.clear();
                let action = match dialog.mode {
                    VaultMode::Create => "vault_create",
                    VaultMode::Unlock => "vault_unlock",
                    _ => "vault_add_key",
                };
                let mut request = json!({"action":action});
                request["passphrase"] = Value::String(std::mem::take(&mut dialog.password));
                dialog.confirmation.zeroize();
                self.send(request);
            }
        }
        if !cancel {
            self.vault_dialog = Some(dialog);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(mut editor) = self.editor.take() {
            let mut cancel = false;
            let mut save = false;
            egui::Modal::new(egui::Id::unique("host_editor")).show(ctx, |ui| {
                ui.set_width(560.0);
                ui.heading(if editor.host.id.is_empty() {
                    "新增主機"
                } else {
                    "編輯主機"
                });
                ui.add_space(10.0);
                ui.add_enabled_ui(!self.busy, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height((ctx.content_rect().height() - 180.0).max(300.0))
                        .show(ui, |ui| {
                            field(
                                ui,
                                "主機名稱",
                                &mut editor.host.name,
                                "例如：Production Web",
                                false,
                            );
                            ui.columns(2, |columns| {
                                field(
                                    &mut columns[0],
                                    "主機位址",
                                    &mut editor.host.address,
                                    "IP 或主機名稱",
                                    false,
                                );
                                field(&mut columns[1], "SSH 埠", &mut editor.port, "22", false);
                            });
                            field(
                                ui,
                                "SSH 使用者名稱",
                                &mut editor.host.username,
                                "使用者名稱",
                                false,
                            );
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_label("目標系統")
                                    .selected_text(&editor.host.os)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut editor.host.os,
                                            "Linux".into(),
                                            "Linux",
                                        );
                                        ui.selectable_value(
                                            &mut editor.host.os,
                                            "BSD".into(),
                                            "BSD",
                                        );
                                    });
                                egui::ComboBox::from_label("登入方式")
                                    .selected_text(if editor.host.auth_type == "key" {
                                        "SSH 私鑰"
                                    } else {
                                        "密碼"
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut editor.host.auth_type,
                                            "password".into(),
                                            "密碼",
                                        );
                                        ui.selectable_value(
                                            &mut editor.host.auth_type,
                                            "key".into(),
                                            "SSH 私鑰",
                                        );
                                    });
                            });
                            ui.checkbox(&mut editor.show_secrets, "顯示憑證內容");
                            if editor.host.auth_type == "password" {
                                field(
                                    ui,
                                    "SSH 密碼",
                                    &mut editor.host.password,
                                    "SSH 登入密碼",
                                    !editor.show_secrets,
                                );
                            } else {
                                ui.label("SSH 私鑰（OpenSSH／PEM）");
                                ui.add(
                                    egui::TextEdit::multiline(&mut editor.host.private_key)
                                        .password(!editor.show_secrets)
                                        .desired_rows(3)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("匯入或貼上私鑰"),
                                );
                                if ui.button("從檔案匯入 SSH Key…").clicked()
                                    && let Some(path) = rfd::FileDialog::new()
                                        .set_title("匯入 SSH 私鑰")
                                        .pick_file()
                                {
                                    let result = std::fs::metadata(&path).and_then(|m| {
                                        if m.len() <= 1024 * 1024 {
                                            std::fs::read_to_string(&path)
                                        } else {
                                            Err(std::io::Error::other("私鑰檔案超過 1 MiB"))
                                        }
                                    });
                                    match result {
                                        Ok(key) => {
                                            editor.host.private_key = key;
                                            editor.error.clear();
                                        }
                                        Err(error) => {
                                            editor.error = format!("無法讀取私鑰：{error}")
                                        }
                                    }
                                }
                                field(
                                    ui,
                                    "私鑰密語",
                                    &mut editor.host.key_passphrase,
                                    "沒有則留空",
                                    !editor.show_secrets,
                                );
                            }
                            ui.separator();
                            field(
                                ui,
                                "sudo 密碼（可選）",
                                &mut editor.host.sudo_password,
                                "留空時沿用 SSH 密碼；Key 登入則需免密 sudo",
                                !editor.show_secrets,
                            );
                            ui.checkbox(
                                &mut editor.host.use_pty,
                                "APT 操作要求 PTY（相當於 ssh -t）",
                            );
                            field(ui, "備註", &mut editor.host.notes, "選填", false);
                            ui.label(
                                RichText::new(if self.encrypted {
                                    "憑證將儲存在已解鎖的 LUKSbox 加密容器。"
                                } else {
                                    "憑證將儲存在本機明文 JSON 檔案。"
                                })
                                .size(12.0)
                                .color(MUTED),
                            );
                            if !editor.error.is_empty() {
                                ui.colored_label(Color32::from_rgb(174, 65, 65), &editor.error);
                            }
                        });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        save = ui.add(primary("儲存主機")).clicked();
                        cancel = ui.button("取消").clicked();
                    });
                });
                if self.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("正在驗證並儲存…");
                    });
                }
            });
            if save {
                match editor.port.trim().parse::<u16>() {
                    Ok(port) if port > 0 => {
                        editor.host.port = port;
                        self.send(json!({"action":"save","host":editor.host}));
                    }
                    _ => editor.error = "SSH 埠必須是 1–65535 的數字".into(),
                }
            }
            if !cancel {
                self.editor = Some(editor);
            }
        }
        if let Some(trust) = &self.trust {
            let mut accept = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique("trust")).show(ctx, |ui| {
                ui.set_width(580.0);
                ui.heading(if trust.old.is_empty() {
                    "確認 SSH 主機指紋"
                } else {
                    "SSH 主機指紋已變更"
                });
                ui.label(&trust.address);
                ui.add_space(10.0);
                ui.label("收到的 SHA-256 指紋");
                ui.monospace(&trust.actual);
                if !trust.old.is_empty() {
                    ui.label("原本信任的指紋");
                    ui.monospace(&trust.old);
                }
                ui.label("請透過可信管道核對。此步驟尚未提交 SSH 帳密。");
                ui.add_enabled_ui(!self.busy, |ui| {
                    ui.horizontal(|ui| {
                        accept = ui.add(primary("信任並儲存")).clicked();
                        cancel = ui.button("取消").clicked();
                    });
                });
            });
            if accept {
                self.send(json!({"action":"trust","id":trust.host_id,"fingerprint":trust.actual}));
            }
            if cancel {
                self.trust = None;
                self.after_trust = None;
                self.status = "未信任主機；連線已取消".into();
            }
        }
        if let Some(id) = self.delete_id.clone() {
            let name = self
                .hosts
                .iter()
                .find(|h| h.id == id)
                .map(|h| h.name.as_str())
                .unwrap_or("主機");
            let mut yes = false;
            let mut no = false;
            egui::Modal::new(egui::Id::unique("delete")).show(ctx, |ui| {
                ui.set_width(400.0);
                ui.heading("刪除主機");
                ui.label(format!("刪除「{name}」的本機連線資料與檢查結果？"));
                ui.add_enabled_ui(!self.busy, |ui| {
                    ui.horizontal(|ui| {
                        yes = ui.button("確定刪除").clicked();
                        no = ui.button("取消").clicked();
                    });
                });
            });
            if yes {
                self.send(json!({"action":"delete","id":id}));
            }
            if no {
                self.delete_id = None;
            }
        }
        self.vault_dialog(ctx);
        if let Some(error) = self.error.clone() {
            let mut close = false;
            egui::Modal::new(egui::Id::unique("error")).show(ctx, |ui| {
                ui.set_width(540.0);
                ui.heading("無法完成操作");
                ui.label(error);
                close = ui.button("關閉").clicked();
            });
            if close {
                self.error = None;
            }
        }
    }
}

impl eframe::App for ManagementApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.process_events();
        let ctx = ui.ctx().clone();
        egui::Panel::bottom("status")
            .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if self.busy {
                        ui.spinner();
                    }
                    ui.label(RichText::new(&self.status).size(12.0).color(MUTED));
                });
            });
        egui::Panel::left("hosts")
            .exact_size(280.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(NAVY).inner_margin(20))
            .show(ui, |ui| {
                ui.add_enabled_ui(!self.busy, |ui| self.sidebar(ui));
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(28))
            .show(ui, |ui| {
                ui.add_enabled_ui(!self.busy, |ui| self.workspace(ui));
            });
        self.dialogs(&ctx);
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        #[cfg(feature = "capture")]
        if let Some(path) = std::env::var_os("MISF_SCREENSHOT_TO") {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            if !self.capture_requested
                && !self.busy
                && self.capture_started.elapsed().as_secs_f32() > 1.0
            {
                self.capture_requested = true;
                let context = ctx.clone();
                ctx.request_screenshot(move |image| {
                    image::save_buffer(
                        std::path::PathBuf::from(path),
                        image.as_raw(),
                        image.width() as u32,
                        image.height() as u32,
                        image::ColorType::Rgba8,
                    )
                    .expect("Failed to save UI screenshot");
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                });
            }
        }
    }
}

fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(8)
        .inner_margin(18)
}

fn wipe_host(host: &mut Host) {
    host.password.zeroize();
    host.private_key.zeroize();
    host.key_passphrase.zeroize();
    host.sudo_password.zeroize();
    host.name.zeroize();
    host.address.zeroize();
    host.username.zeroize();
    host.notes.zeroize();
    if let Some(result) = &mut host.last_check {
        result.output.zeroize();
        result.error.zeroize();
    }
}
fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE).strong()).fill(BLUE)
}
fn stat(ui: &mut egui::Ui, title: &str, value: &str) {
    card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.label(RichText::new(title).size(12.0).color(MUTED));
        ui.add_space(6.0);
        ui.label(RichText::new(value).size(22.0).strong());
    });
}
fn field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str, secret: bool) {
    ui.label(RichText::new(label).size(12.0).strong());
    ui.add(
        egui::TextEdit::singleline(value)
            .password(secret)
            .desired_width(f32::INFINITY)
            .hint_text(hint),
    );
}
fn format_time(value: &str) -> String {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Local).format("%m/%d %H:%M").to_string())
        .unwrap_or_else(|_| value.into())
}
