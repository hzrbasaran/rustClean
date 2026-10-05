//! The help screen (`?`): every key, the current screen's first.

use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, Browser, Screen, View};
use crate::lists::ResultList;

use super::format::wrap_indented;
use super::style::Themed;
use super::theme::theme;

/// The screen the keys belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    List,
    Map,
    Results,
    Apps,
    Basket,
    Dashboard,
    Menu,
    Dialogs,
    Tools,
    System,
    Log,
    Disks,
    Scanning,
}

impl Topic {
    /// Every topic, in the order the help lists the other screens.
    pub const ALL: [Topic; 13] = [
        Topic::List,
        Topic::Map,
        Topic::Results,
        Topic::Apps,
        Topic::Basket,
        Topic::Dashboard,
        Topic::Menu,
        Topic::Dialogs,
        Topic::Tools,
        Topic::System,
        Topic::Log,
        Topic::Disks,
        Topic::Scanning,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Topic::List => t!("Klasör listesi", "Folder list"),
            Topic::Map => t!("Harita (treemap)", "Map (treemap)"),
            Topic::Results => t!("Raporlar ve arama sonuçları", "Reports and search results"),
            Topic::Apps => t!("Uygulamalar raporu", "Applications report"),
            Topic::Basket => t!("Sepet", "Basket"),
            Topic::Dashboard => t!("Özet", "Summary"),
            Topic::Menu => t!(
                "Rapor menüsü ve kayıt seçimi",
                "Report menu and saved scans"
            ),
            Topic::Dialogs => t!(
                "Onay, kaldırma ve hata pencereleri",
                "Confirm, uninstall and error dialogs"
            ),
            Topic::Tools => t!("Geliştirici araçları", "Developer tools"),
            Topic::System => t!("Sistem verileri", "System data"),
            Topic::Log => t!("Silme kaydı", "Deletion log"),
            Topic::Disks => t!("Disk listesi", "Disk list"),
            Topic::Scanning => t!("Tarama", "Scanning"),
        }
    }

    /// The keys of this screen: (keys, what they do).
    pub fn keys(self) -> Vec<(&'static str, &'static str)> {
        let moving = [
            ("↑↓  j k", t!("gez", "move")),
            ("PgUp PgDn", t!("sayfa sayfa", "page up / down")),
            ("Home End  g G", t!("ilk / son satır", "first / last row")),
        ];
        let cleaning = [
            (
                "Space",
                t!("sepete ekle / çıkar", "add to / remove from the basket"),
            ),
            ("S", t!("sepeti göster", "show the basket")),
            (
                "x  Delete",
                t!(
                    "çöpe taşı (önce sorar; sepet doluysa sepeti)",
                    "move to the trash (asks first; the basket if it has entries)"
                ),
            ),
        ];
        let mut keys: Vec<(&str, &str)> = Vec::new();
        match self {
            Topic::List => {
                keys.extend(moving);
                keys.extend([
                    ("Enter  →  l", t!("klasöre gir", "open the folder")),
                    ("⌫  ←  h  Esc", t!("üst klasör", "parent folder")),
                    (
                        "s",
                        t!(
                            "sırala: boyut, ad, dosya sayısı, tarih",
                            "sort: size, name, file count, date"
                        ),
                    ),
                    (
                        "a",
                        t!("görünen / diskteki boyut", "apparent / on-disk size"),
                    ),
                    ("t", t!("harita görünümü", "map view")),
                    (
                        "w",
                        t!(
                            "haritayı HTML sayfası olarak kaydet",
                            "save the treemap as an HTML page"
                        ),
                    ),
                    (
                        "/",
                        t!(
                            "ada göre ara (* ve ? joker karakter)",
                            "find by name (* and ? are wildcards)"
                        ),
                    ),
                    ("i", t!("bu klasörün özeti", "summary of this folder")),
                    ("m", t!("raporlar menüsü", "reports menu")),
                ]);
                keys.extend(cleaning);
                keys.extend([
                    ("R", t!("bu klasörü yeniden tara", "rescan this folder")),
                    ("r", t!("hepsini yeniden tara", "rescan everything")),
                    ("d", t!("disk listesine dön", "back to the disk list")),
                ]);
            }
            Topic::Map => {
                keys.extend([
                    ("←↑↓→  h j k l", t!("blok seç", "select a block")),
                    (
                        "Enter",
                        t!(
                            "klasöre gir (\"diğer\": listede gör)",
                            "open the folder (\"other\": show in the list)"
                        ),
                    ),
                    ("⌫", t!("üst klasör", "parent folder")),
                    ("c", t!("renk: türe / yaşa göre", "color: by type / by age")),
                    ("t  Esc", t!("listeye dön", "back to the list")),
                    (
                        "w",
                        t!(
                            "HTML sayfası olarak kaydet (tarayıcıda yakınlaştırılır)",
                            "save as an HTML page (zoomable in a browser)"
                        ),
                    ),
                    ("m", t!("raporlar menüsü", "reports menu")),
                ]);
                keys.extend(cleaning);
                keys.push(("R", t!("bu klasörü yeniden tara", "rescan this folder")));
            }
            Topic::Results | Topic::Apps => {
                keys.extend(moving);
                keys.extend([
                    (
                        "Enter  →  l",
                        t!(
                            "grubu aç / konumuna git",
                            "open the group / go to its location"
                        ),
                    ),
                    ("Esc  ⌫  ←  h", t!("geri", "back")),
                    (
                        "Space",
                        t!(
                            "sepete ekle / çıkar (kopyalarda en eski kalır)",
                            "add to / remove from the basket (copies keep the oldest)"
                        ),
                    ),
                    ("t", t!("hepsini sepete ekle", "add all to the basket")),
                    (
                        "f",
                        t!(
                            "yaş filtresi: 30, 90, 180, 365 gün",
                            "age filter: 30, 90, 180, 365 days"
                        ),
                    ),
                    ("S", t!("sepeti göster", "show the basket")),
                    (
                        "x  Delete",
                        t!("çöpe taşı (önce sorar)", "move to the trash (asks first)"),
                    ),
                    ("/", t!("yeni arama", "new search")),
                    (
                        "a",
                        t!("görünen / diskteki boyut", "apparent / on-disk size"),
                    ),
                    ("m", t!("raporlar menüsü", "reports menu")),
                ]);
                if self == Topic::Apps {
                    keys.insert(
                        0,
                        (
                            "u",
                            t!(
                                "uygulamayı verileriyle kaldır",
                                "uninstall the app with its data"
                            ),
                        ),
                    );
                }
            }
            Topic::Basket => {
                keys.extend(moving);
                keys.extend([
                    ("Space", t!("sepetten çıkar", "remove from the basket")),
                    ("c", t!("sepeti boşalt", "empty the basket")),
                    (
                        "x  Delete",
                        t!(
                            "hepsini çöpe taşı (önce sorar)",
                            "move all to the trash (asks first)"
                        ),
                    ),
                    ("Enter", t!("konumuna git", "go to its location")),
                    ("Esc  ⌫", t!("geri", "back")),
                ]);
            }
            Topic::Dashboard => keys.extend([
                ("Tab", t!("listeler arasında geç", "switch list")),
                ("↑↓  j k", t!("gez", "move")),
                ("Home End  g G", t!("ilk / son satır", "first / last row")),
                ("Enter  →  l", t!("konumuna git", "go to its location")),
                (
                    "Space",
                    t!("sepete ekle / çıkar", "add to / remove from the basket"),
                ),
                (
                    "x  Delete",
                    t!("çöpe taşı (önce sorar)", "move to the trash (asks first)"),
                ),
                ("S", t!("sepeti göster", "show the basket")),
                (
                    "a",
                    t!("görünen / diskteki boyut", "apparent / on-disk size"),
                ),
                ("Esc  i  ⌫", t!("geri", "back")),
            ]),
            Topic::Menu => keys.extend([
                ("↑↓  j k", t!("seç", "select")),
                ("Enter", t!("çalıştır / karşılaştır", "run / compare")),
                ("Esc  m", t!("kapat", "close")),
            ]),
            Topic::Dialogs => keys.extend([
                (
                    t!("e  y", "y  e"),
                    t!(
                        "evet: çöpe taşı / kaldır",
                        "yes: move to the trash / uninstall"
                    ),
                ),
                (
                    t!("h  n  Esc", "n  h  Esc"),
                    t!(
                        "vazgeç (silme onayında her tuş)",
                        "cancel (any key when deleting)"
                    ),
                ),
                (
                    "↑↓  Space  t",
                    t!(
                        "kaldırmada: gez, işaretle, tümü",
                        "uninstall: move, check, all"
                    ),
                ),
                (
                    "↑↓  PgUp PgDn",
                    t!("hata listesini kaydır", "scroll the error list"),
                ),
            ]),
            Topic::Tools => keys.extend([
                ("↑↓  j k", t!("araç seç", "select a tool")),
                (
                    "Enter",
                    t!("ne temizleneceğini seç", "choose what to clean"),
                ),
                ("Space", t!("işaretle (seçimde)", "check (when choosing)")),
                (
                    t!("evet + Enter", "yes + Enter"),
                    t!(
                        "veri kaybı olan işlemleri onayla",
                        "confirm actions that lose data"
                    ),
                ),
                ("r", t!("yeniden ölç", "measure again")),
                ("m", t!("raporlar menüsü", "reports menu")),
                ("Esc  ⌫  ←", t!("geri / vazgeç", "back / cancel")),
            ]),
            Topic::System => keys.extend([
                ("r", t!("yenile", "refresh")),
                ("m", t!("raporlar menüsü", "reports menu")),
                ("Esc  ⌫  ←", t!("geri", "back")),
            ]),
            Topic::Log => keys.extend([
                ("↑↓  j k", t!("kaydır", "scroll")),
                ("PgUp PgDn", t!("sayfa sayfa", "page up / down")),
                ("Home End  g G", t!("baş / son", "top / end")),
                ("r", t!("yenile", "refresh")),
                ("m", t!("raporlar menüsü", "reports menu")),
                ("Esc  ⌫  ←", t!("geri", "back")),
            ]),
            Topic::Disks => keys.extend([
                ("↑↓  j k", t!("disk seç", "select a disk")),
                ("Enter  →  l", t!("tara", "scan")),
                ("r", t!("listeyi yenile", "refresh the list")),
                ("Esc", t!("çık", "quit")),
            ]),
            Topic::Scanning => keys.push(("Esc", t!("taramayı iptal et", "cancel the scan"))),
        }
        keys
    }
}

/// Keys that work on every screen.
pub fn global_keys() -> Vec<(&'static str, &'static str)> {
    vec![
        ("?", t!("bu yardım", "this help")),
        ("L", t!("Türkçe ↔ English", "English ↔ Türkçe")),
        (
            "T",
            t!(
                "tema: koyu, açık, renk körü",
                "theme: dark, light, color-blind"
            ),
        ),
        ("q  Ctrl-C", t!("çık", "quit")),
    ]
}

/// The screen the user is on.
pub fn topic(app: &App) -> Topic {
    match app.screen {
        Screen::DiskSelect => Topic::Disks,
        Screen::Scanning => Topic::Scanning,
        Screen::Browser => app.browser.as_ref().map_or(Topic::List, browser_topic),
    }
}

fn browser_topic(b: &Browser) -> Topic {
    if b.failures.is_some() || b.confirm.is_some() || b.uninstall.is_some() {
        Topic::Dialogs
    } else if b.report_menu.is_some() || b.snapshot_picker.is_some() {
        Topic::Menu
    } else if b.tools.is_some() {
        Topic::Tools
    } else if b.deletion_log.is_some() {
        Topic::Log
    } else if b.system.is_some() {
        Topic::System
    } else if b.dashboard.is_some() {
        Topic::Dashboard
    } else if b.results.as_ref().is_some_and(ResultList::is_basket) {
        Topic::Basket
    } else if b.apps_report().is_some() {
        Topic::Apps
    } else if b.results.is_some() {
        Topic::Results
    } else if b.view == View::Map {
        Topic::Map
    } else {
        Topic::List
    }
}

/// The help lines: this screen, then the keys of every screen, then the
/// others. Descriptions wider than `width` continue under themselves.
fn lines(current: Topic, key_w: usize, width: usize) -> Vec<Line<'static>> {
    let indent = " ".repeat(key_w + 2);
    let section = |title: String, keys: Vec<(&'static str, &'static str)>, now: bool| {
        let heading = if now {
            Line::from(Span::raw(title).accent().bold())
        } else {
            Line::from(Span::raw(title).normal().bold())
        };
        let mut out = vec![heading];
        for (k, desc) in keys {
            let wrapped = wrap_indented(desc, width, &indent, &indent);
            for (i, l) in wrapped.into_iter().enumerate() {
                let text = l[indent.len()..].to_string();
                let key = if i == 0 {
                    format!(" {k:<key_w$} ")
                } else {
                    indent.clone()
                };
                out.push(Line::from(vec![
                    Span::raw(key).accent(),
                    Span::raw(text).normal(),
                ]));
            }
        }
        out.push(Line::from(""));
        out
    };
    let mut out = section(
        tf!("▶ Bu ekran: {}", "▶ This screen: {}", current.title()),
        current.keys(),
        true,
    );
    out.extend(section(
        t!("Her ekranda", "On every screen").into(),
        global_keys(),
        false,
    ));
    for t in Topic::ALL {
        // Results and the Apps report share their keys.
        let shown = t == current || (current == Topic::Apps && t == Topic::Results);
        if !shown {
            out.extend(section(t.title().into(), t.keys(), false));
        }
    }
    out.pop();
    out
}

/// Draws the help over the screen. `scroll` is kept within the text.
pub(super) fn render_help(f: &mut Frame<'_>, app: &App, scroll: &mut u16) {
    let area = f.area();
    let width = area.width.saturating_sub(4).min(84);
    let height = area.height.saturating_sub(2);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + 1,
        width,
        height,
    };
    let key_w = Topic::ALL
        .iter()
        .flat_map(|t| t.keys())
        .chain(global_keys())
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    let lines = lines(topic(app), key_w, usize::from(width.saturating_sub(2)));
    let visible = usize::from(height.saturating_sub(2));
    let max = u16::try_from(lines.len().saturating_sub(visible)).unwrap_or(u16::MAX);
    *scroll = (*scroll).min(max);
    let footer = if max > 0 {
        t!(
            " ↑↓ PgUp PgDn: kaydır · Esc / ?: kapat ",
            " ↑↓ PgUp PgDn: scroll · Esc / ?: close "
        )
    } else {
        t!(" Esc / ?: kapat ", " Esc / ?: close ")
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).scroll((*scroll, 0)).block(
            Block::bordered()
                .title(Span::raw(t!(" Tuşlar ", " Keys ")).normal().bold())
                .title_bottom(Line::from(footer).centered().muted())
                .border_style(ratatui::style::Style::new().fg(theme().accent)),
        ),
        rect,
    );
}
