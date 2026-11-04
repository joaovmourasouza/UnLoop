//! Configurações: aparência, moeda, lema e dados (backup/restauração/CSV).

use adw::prelude::*;
use gtk::gio;
use unloop_core::backup::{export_backup, export_diary_csv, import_backup};
use unloop_core::presets::CURRENCY_CODES;

use crate::app::Shared;
use crate::{widgets, VERSION};

const THEMES: [(&str, &str); 3] = [("system", "Seguir o sistema"), ("light", "Claro"), ("dark", "Escuro")];

pub fn apply_theme(theme: &str) {
    let scheme = match theme {
        "light" => adw::ColorScheme::ForceLight,
        "dark" => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
}

pub fn page(ctx: &Shared) -> adw::NavigationPage {
    let prefs = adw::PreferencesPage::new();

    // Aparência
    let appearance = widgets::group("Aparência", None);
    let theme_labels: Vec<&str> = THEMES.iter().map(|t| t.1).collect();
    let theme_row = adw::ComboRow::builder()
        .title("Tema")
        .model(&gtk::StringList::new(&theme_labels))
        .build();
    let current_theme = ctx.setting("theme", "system");
    theme_row.set_selected(THEMES.iter().position(|t| t.0 == current_theme).unwrap_or(0) as u32);
    let c = ctx.clone();
    theme_row.connect_selected_notify(move |row| {
        let key = THEMES.get(row.selected() as usize).map_or("system", |t| t.0);
        if c.check(c.db.set_setting("theme", key)).is_some() {
            apply_theme(key);
        }
    });
    appearance.add(&theme_row);

    let motto = adw::EntryRow::builder().title("Lema pessoal (aparece na tela inicial)").build();
    motto.set_text(&ctx.setting("motto", ""));
    motto.set_show_apply_button(true);
    let c = ctx.clone();
    motto.connect_apply(move |row| {
        let text = row.text().trim().to_string();
        if c.check(c.db.set_setting("motto", &text)).is_some() {
            c.changed();
            c.toast("Lema salvo.");
        }
    });
    appearance.add(&motto);

    let currency_row = adw::ComboRow::builder()
        .title("Moeda")
        .model(&gtk::StringList::new(CURRENCY_CODES))
        .build();
    let current_currency = ctx.currency();
    currency_row.set_selected(CURRENCY_CODES.iter().position(|c| *c == current_currency).unwrap_or(0) as u32);
    let c = ctx.clone();
    currency_row.connect_selected_notify(move |row| {
        if let Some(code) = CURRENCY_CODES.get(row.selected() as usize) {
            if c.check(c.db.set_setting("currency", code)).is_some() {
                c.changed();
            }
        }
    });
    appearance.add(&currency_row);
    prefs.add(&appearance);

    // Dados
    let data = widgets::group(
        "Dados",
        Some("Tudo fica neste computador. Faça backups de vez em quando e guarde-os onde quiser."),
    );
    let export_row = action_row("Exportar backup (JSON)", "Arquivo completo, para guardar ou levar a outro computador");
    let c = ctx.clone();
    export_row.connect_activated(move |_| export_json(&c));
    data.add(&export_row);

    let import_row = action_row("Restaurar backup", "Do UnLoop ou do Quitzilla (backup em arquivo). Substitui os dados atuais.");
    let c = ctx.clone();
    import_row.connect_activated(move |_| import_json(&c));
    data.add(&import_row);

    let csv_row = action_row("Exportar diário (CSV)", "Para análise em planilhas");
    let c = ctx.clone();
    csv_row.connect_activated(move |_| export_csv(&c));
    data.add(&csv_row);

    let path_row = adw::ActionRow::builder()
        .title("Banco de dados")
        .subtitle(widgets::esc(&ctx.db_path.display().to_string()))
        .subtitle_selectable(true)
        .build();
    data.add(&path_row);
    prefs.add(&data);

    let about = widgets::group("Sobre", None);
    let about_row = action_row(&format!("UnLoop {VERSION}"), "Sem conta, sem nuvem, sem anúncios e sem rastreamento.");
    about_row.set_action_name(Some("app.about"));
    about.add(&about_row);
    prefs.add(&about);

    let header = adw::HeaderBar::new();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&prefs));
    adw::NavigationPage::builder().title("Configurações").child(&toolbar).build()
}

fn action_row(title: &str, subtitle: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(widgets::esc(title))
        .subtitle(widgets::esc(subtitle))
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row
}

fn file_filter(name: &str, suffix: &str) -> gio::ListStore {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(name));
    filter.add_suffix(suffix);
    let store = gio::ListStore::new::<gtk::FileFilter>();
    store.append(&filter);
    store
}

fn export_json(ctx: &Shared) {
    let dialog = gtk::FileDialog::builder()
        .title("Exportar backup")
        .initial_name(format!("unloop-backup-{}.json", widgets::file_stamp()))
        .modal(true)
        .build();
    dialog.set_filters(Some(&file_filter("Backup JSON", "json")));
    let c = ctx.clone();
    dialog.save(Some(&ctx.window), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else {
            c.toast("Local inválido.");
            return;
        };
        let Some(json) = c.check(export_backup(&c.db, VERSION)) else { return };
        let text = serde_json::to_string_pretty(&json).unwrap_or_default();
        if c.check(std::fs::write(&path, text)).is_some() {
            c.toast(&format!("Backup salvo em {}", path.display()));
        }
    });
}

fn export_csv(ctx: &Shared) {
    let dialog = gtk::FileDialog::builder()
        .title("Exportar diário")
        .initial_name(format!("unloop-diario-{}.csv", widgets::file_stamp()))
        .modal(true)
        .build();
    dialog.set_filters(Some(&file_filter("CSV", "csv")));
    let c = ctx.clone();
    dialog.save(Some(&ctx.window), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        let Some(csv) = c.check(export_diary_csv(&c.db)) else { return };
        if c.check(std::fs::write(&path, csv)).is_some() {
            c.toast(&format!("CSV salvo em {}", path.display()));
        }
    });
}

fn import_json(ctx: &Shared) {
    let dialog = gtk::FileDialog::builder().title("Restaurar backup").modal(true).build();
    dialog.set_filters(Some(&file_filter("Backup JSON", "json")));
    let c = ctx.clone();
    dialog.open(Some(&ctx.window), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        let Some(text) = c.check(std::fs::read_to_string(&path)) else { return };
        let Some(json) = c.check(serde_json::from_str::<serde_json::Value>(&text)) else { return };
        let c2 = c.clone();
        widgets::confirm(
            &c,
            "Restaurar backup?",
            "Todos os hábitos atuais serão substituídos pelo conteúdo do arquivo.",
            "Restaurar",
            true,
            move || {
                if let Some(summary) = c2.check(import_backup(&c2.db, &json, true)) {
                    c2.changed();
                    let origin = if summary.from_quitzilla { "Backup do Quitzilla" } else { "Backup" };
                    widgets::info(&c2, &format!("{origin} importado"), &summary.to_string());
                }
            },
        );
    });
}
