//! Estado compartilhado da aplicação, janela principal, ações e relógio.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use unloop_core::db::Db;
use unloop_core::model::COLORS;

use crate::{home, settings, APP_ID, VERSION};

pub type Shared = Rc<Ctx>;

type Refresher = Box<dyn Fn() -> bool>;
type Ticker = Box<dyn Fn(i64) -> bool>;

/// Contexto compartilhado por todas as telas.
///
/// - `on_refresh`: callbacks chamados após qualquer escrita no banco (`changed`).
/// - `live`: callbacks chamados a cada segundo com o instante atual.
///
/// Os callbacks devem guardar só referências fracas aos widgets e retornar
/// `false` quando eles não existirem mais; nesse caso são descartados.
pub struct Ctx {
    pub db: Db,
    pub db_path: PathBuf,
    pub window: adw::ApplicationWindow,
    pub nav: adw::NavigationView,
    toasts: adw::ToastOverlay,
    refreshers: RefCell<Vec<Refresher>>,
    tickers: RefCell<Vec<Ticker>>,
}

impl Ctx {
    pub fn toast(&self, message: &str) {
        // Títulos de toast são Pango markup: escapa caminhos e mensagens de erro.
        self.toasts.add_toast(adw::Toast::new(&glib::markup_escape_text(message)));
    }

    /// Mostra o erro num toast e devolve `None`.
    pub fn check<T, E: std::fmt::Display>(&self, result: Result<T, E>) -> Option<T> {
        match result {
            Ok(v) => Some(v),
            Err(e) => {
                self.toast(&format!("Erro: {e}"));
                None
            }
        }
    }

    pub fn setting(&self, key: &str, default: &str) -> String {
        self.db.setting(key).ok().flatten().unwrap_or_else(|| default.to_string())
    }

    pub fn currency(&self) -> String {
        self.setting("currency", "BRL")
    }

    pub fn on_refresh(&self, f: impl Fn() -> bool + 'static) {
        self.refreshers.borrow_mut().push(Box::new(f));
    }

    /// Executa `f` agora e a cada segundo enquanto retornar `true`.
    pub fn live(&self, f: impl Fn(i64) -> bool + 'static) {
        if f(unloop_core::now_ms()) {
            self.tickers.borrow_mut().push(Box::new(f));
        }
    }

    /// Avisa todas as telas que os dados mudaram.
    pub fn changed(&self) {
        let list = std::mem::take(&mut *self.refreshers.borrow_mut());
        let mut keep: Vec<Refresher> = list.into_iter().filter(|f| f()).collect();
        let mut current = self.refreshers.borrow_mut();
        keep.append(&mut current);
        *current = keep;
    }

    fn tick(&self) {
        let now = unloop_core::now_ms();
        let list = std::mem::take(&mut *self.tickers.borrow_mut());
        let mut keep: Vec<Ticker> = list.into_iter().filter(|f| f(now)).collect();
        let mut current = self.tickers.borrow_mut();
        keep.append(&mut current);
        *current = keep;
    }
}

pub fn activate(app: &adw::Application) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }

    load_css();

    // Dentro do Flatpak: ~/.var/app/<APP_ID>/data/unloop/unloop.db
    let data_dir = glib::user_data_dir().join("unloop");
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("unloop: não foi possível criar {}: {e}", data_dir.display());
    }
    let db_path = data_dir.join("unloop.db");
    let db = match Db::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("unloop: não foi possível abrir o banco {}: {e}", db_path.display());
            app.quit();
            return;
        }
    };

    let nav = adw::NavigationView::new();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&nav));
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("UnLoop")
        .icon_name(APP_ID)
        .default_width(980)
        .default_height(760)
        .width_request(360)
        .height_request(480)
        .content(&toasts)
        .build();

    let ctx: Shared = Rc::new(Ctx {
        db,
        db_path,
        window: window.clone(),
        nav: nav.clone(),
        toasts,
        refreshers: RefCell::new(Vec::new()),
        tickers: RefCell::new(Vec::new()),
    });

    settings::apply_theme(&ctx.setting("theme", "system"));
    setup_actions(app, &ctx);
    nav.push(&home::page(&ctx));

    let weak = Rc::downgrade(&ctx);
    glib::timeout_add_seconds_local(1, move || match weak.upgrade() {
        Some(ctx) => {
            ctx.tick();
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });

    // Mantém o contexto vivo enquanto a janela existir.
    let keep_alive = ctx.clone();
    window.connect_close_request(move |_| {
        let _ = &keep_alive;
        glib::Propagation::Proceed
    });

    window.present();
}

fn setup_actions(app: &adw::Application, ctx: &Shared) {
    let preferences = gio::SimpleAction::new("preferences", None);
    let c = ctx.clone();
    preferences.connect_activate(move |_, _| c.nav.push(&settings::page(&c)));
    app.add_action(&preferences);

    let about = gio::SimpleAction::new("about", None);
    let c = ctx.clone();
    about.connect_activate(move |_, _| show_about(&c));
    app.add_action(&about);

    let quit = gio::SimpleAction::new("quit", None);
    let a = app.clone();
    quit.connect_activate(move |_, _| a.quit());
    app.add_action(&quit);

    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("app.preferences", &["<Control>comma"]);
}

fn show_about(ctx: &Shared) {
    adw::AboutWindow::builder()
        .transient_for(&ctx.window)
        .modal(true)
        .application_name("UnLoop")
        .application_icon(APP_ID)
        .version(VERSION)
        .developer_name("João Moura")
        .license_type(gtk::License::Gpl30)
        .website("https://github.com/joaovmourasouza/UnLoop")
        .comments("Rastreador de hábitos offline. Seus dados ficam só neste computador: sem conta, sem nuvem, sem anúncios e sem rastreamento.")
        .build()
        .present();
}

fn load_css() {
    let mut css = String::from(
        "
        .avatar { border-radius: 9999px; min-width: 44px; min-height: 44px; }
        .avatar label { font-size: 22px; }
        .avatar.large { min-width: 72px; min-height: 72px; }
        .avatar.large label { font-size: 38px; }
        .counter { font-size: 40px; font-weight: 800; font-feature-settings: \"tnum\"; }
        .counter-small { font-size: 18px; font-weight: 700; font-feature-settings: \"tnum\"; }
        .trophy { padding: 14px; }
        .trophy .emoji { font-size: 40px; }
        .trophy.locked { opacity: 0.45; }
        .affirmation { font-style: italic; font-size: 17px; }
        progressbar.thick trough, progressbar.thick progress { min-height: 10px; border-radius: 5px; }
        ",
    );
    for c in COLORS {
        css.push_str(&format!(
            ".habit-color-{code} {{ background-color: #{hex}; color: white; }}\n\
             progressbar.bar-color-{code} progress {{ background-color: #{hex}; }}\n",
            code = c.code,
            hex = c.hex
        ));
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}
