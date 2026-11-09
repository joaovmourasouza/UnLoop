//! Moldura do app no estilo TickTick: trilho lateral com as seções
//! (Hoje, Tarefas, Hábitos, Foco, Configurações) e a área de conteúdo
//! (`Ctx::nav`), cuja raiz é trocada ao escolher uma seção.

use adw::prelude::*;
use gtk::gio;

use crate::app::Shared;
use crate::widgets;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Today,
    Tasks,
    Habits,
    Focus,
    Settings,
}

impl Section {
    pub const MAIN: [Section; 4] = [Section::Today, Section::Tasks, Section::Habits, Section::Focus];

    fn icon(self) -> &'static str {
        match self {
            Section::Today => "weather-clear-symbolic",
            Section::Tasks => "object-select-symbolic",
            Section::Habits => "starred-symbolic",
            Section::Focus => "alarm-symbolic",
            Section::Settings => "emblem-system-symbolic",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Section::Today => "Hoje",
            Section::Tasks => "Tarefas",
            Section::Habits => "Hábitos",
            Section::Focus => "Foco",
            Section::Settings => "Ajustes",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Section::Today => "Hoje (Ctrl+1)",
            Section::Tasks => "Tarefas e listas (Ctrl+2 ou Ctrl+T)",
            Section::Habits => "Hábitos (Ctrl+3)",
            Section::Focus => "Foco / Pomodoro (Ctrl+4 ou Ctrl+F)",
            Section::Settings => "Configurações (Ctrl+,)",
        }
    }

    fn page(self, ctx: &Shared) -> adw::NavigationPage {
        match self {
            Section::Today => crate::home::page(ctx),
            Section::Tasks => crate::tasks_page::page(ctx),
            Section::Habits => crate::home::habits_page(ctx),
            Section::Focus => crate::focus::page(ctx),
            Section::Settings => crate::settings::page(ctx),
        }
    }
}

/// Abre a seção na raiz (descarta as páginas empilhadas) e destaca o botão no trilho.
pub fn go(ctx: &Shared, section: Section) {
    ctx.section.set(section);
    for (s, button) in ctx.rail.borrow().iter() {
        if *s == section {
            button.add_css_class("active");
        } else {
            button.remove_css_class("active");
        }
    }
    ctx.nav.replace(&[section.page(ctx)]);
}

fn rail_button(ctx: &Shared, section: Section) -> (gtk::Button, gtk::Label) {
    let icon = gtk::Image::from_icon_name(section.icon());
    icon.set_pixel_size(20);
    let label = gtk::Label::new(Some(section.label()));
    label.add_css_class("rail-label");
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 3);
    inner.append(&icon);
    inner.append(&label);
    let button = gtk::Button::builder().child(&inner).tooltip_text(section.tooltip()).build();
    button.add_css_class("flat");
    button.add_css_class("rail-button");
    let c = ctx.clone();
    button.connect_clicked(move |_| go(&c, section));
    ctx.rail.borrow_mut().push((section, button.clone()));
    (button, label)
}

/// Trilho lateral. Deve ser chamado uma vez, depois de criar o contexto.
pub fn rail(ctx: &Shared) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.add_css_class("rail");

    let logo = gtk::Image::from_icon_name(crate::APP_ID);
    logo.set_pixel_size(32);
    logo.set_margin_top(10);
    logo.set_margin_bottom(10);
    logo.set_tooltip_text(Some("UnLoop"));
    column.append(&logo);

    let mut labels = Vec::new();
    for section in Section::MAIN {
        let (button, label) = rail_button(ctx, section);
        column.append(&button);
        labels.push(label);
    }

    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    column.append(&spacer);

    let (settings, label) = rail_button(ctx, Section::Settings);
    column.append(&settings);
    labels.push(label);

    let menu = gio::Menu::new();
    menu.append(Some("Sobre o UnLoop"), Some("app.about"));
    menu.append(Some("Sair"), Some("app.quit"));
    let more = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .tooltip_text("Mais")
        .direction(gtk::ArrowType::Right)
        .halign(gtk::Align::Center)
        .build();
    more.add_css_class("flat");
    more.set_margin_bottom(10);
    column.append(&more);

    // Timer em andamento aparece no lugar do rótulo "Foco".
    if let Some(focus_label) = labels.get(3).cloned() {
        let (c, weak) = (ctx.clone(), focus_label.downgrade());
        ctx.live(move |now| {
            let Some(label) = weak.upgrade() else { return false };
            let timer = c.timer.borrow();
            if timer.is_idle() {
                label.set_label(Section::Focus.label());
                label.remove_css_class("rail-timer");
            } else {
                let settings = c.pomodoro_settings.borrow();
                label.set_label(&widgets::clock(timer.display_ms(now, &settings)));
                label.add_css_class("rail-timer");
            }
            true
        });
    }

    // Janela estreita: trilho só com ícones.
    if let Ok(condition) = adw::BreakpointCondition::parse("max-width: 480sp") {
        let breakpoint = adw::Breakpoint::new(condition);
        for label in &labels {
            breakpoint.add_setter(label, "visible", Some(&false.to_value()));
        }
        ctx.window.add_breakpoint(breakpoint);
    }

    // Arrastar o trilho move a janela (ele fica fora da barra de título).
    gtk::WindowHandle::builder().child(&column).build().upcast()
}

/// Cabeçalho padrão das páginas-raiz das seções: título grande + subtítulo opcional.
pub fn header(title: &str, subtitle: &str) -> adw::HeaderBar {
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(title, subtitle)));
    header
}
