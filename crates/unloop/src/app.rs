//! Estado compartilhado da aplicação, janela principal, ações e relógio.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use unloop_core::checkin::{self, NaiveDate};
use unloop_core::db::Db;
use unloop_core::model::COLORS;
use unloop_core::pomodoro::{self, Credit, FocusRecord, Phase, Timer};

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
    /// Timer de foco (continua rodando enquanto se navega pelo app).
    pub timer: RefCell<Timer>,
    pub pomodoro_settings: RefCell<pomodoro::Settings>,
    last_day: Cell<NaiveDate>,
    /// `PRAGMA data_version` visto por último: muda quando a CLI (ou outro processo) grava.
    data_version: Cell<i64>,
}

/// Agenda `changed()` para depois do sinal atual (evita remover o widget que emitiu o sinal).
pub fn changed_later(ctx: &Shared) {
    let c = ctx.clone();
    glib::idle_add_local_once(move || c.changed());
}

impl Ctx {
    /// Salva a sessão de foco e credita o hábito vinculado (minutos ou check do dia).
    pub fn save_focus(&self, record: &FocusRecord) {
        let mut r = record.clone();
        let habit = r.habit_id.and_then(|id| self.db.habit(id).ok().flatten());
        if habit.is_none() {
            r.habit_id = None;
        }
        if self.check(self.db.insert_focus(&r)).is_none() {
            return;
        }
        let Some(h) = habit else { return };
        let day = checkin::day_key(checkin::local_date(r.ended_at));
        match pomodoro::focus_credit(&h, r.duration_ms) {
            Credit::AddProgress(amount) => {
                if let Some(total) = self.check(self.db.add_progress(h.id, &day, amount)) {
                    self.toast(&format!(
                        "{}: +{} {} (hoje: {})",
                        h.name,
                        unloop_core::format::format_number(amount),
                        h.target_unit,
                        unloop_core::format::format_number(total)
                    ));
                }
            }
            Credit::Check => {
                if self.check(self.db.set_checkin(h.id, &day, true)).is_some() {
                    self.toast(&format!("{}: marcado como feito hoje", h.name));
                }
            }
            Credit::None => {}
        }
    }

    fn notify_phase_end(&self, finished: Phase) {
        let next = self.timer.borrow().phase;
        let (title, body) = if finished.is_focus() {
            ("Foco concluído 🍅", format!("Bom trabalho! Agora: {}.", next.label().to_lowercase()))
        } else {
            ("Pausa terminada", "Hora de voltar ao foco.".to_string())
        };
        if let Some(app) = self.window.application() {
            let n = gio::Notification::new(title);
            n.set_body(Some(&body));
            app.send_notification(Some("unloop-focus"), &n);
        }
        if let Some(display) = gdk::Display::default() {
            display.beep();
        }
    }

    fn process_timer(&self, now: i64) {
        let settings = self.pomodoro_settings.borrow().clone();
        let result = self.timer.borrow_mut().tick(now, &settings);
        if let Some((finished, record)) = result {
            if let Some(r) = record {
                self.save_focus(&r);
            }
            self.notify_phase_end(finished);
            self.changed();
        }
    }
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
        self.process_timer(now);
        let today = checkin::today();
        let version = self.db.data_version().unwrap_or(0);
        if today != self.last_day.get() || version != self.data_version.get() {
            self.last_day.set(today);
            self.data_version.set(version);
            self.changed();
        }
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

    let pomodoro_settings = pomodoro::Settings::load(&db);
    let data_version = db.data_version().unwrap_or(0);
    let ctx: Shared = Rc::new(Ctx {
        db,
        db_path,
        window: window.clone(),
        nav: nav.clone(),
        toasts,
        refreshers: RefCell::new(Vec::new()),
        tickers: RefCell::new(Vec::new()),
        timer: RefCell::new(Timer::default()),
        pomodoro_settings: RefCell::new(pomodoro_settings),
        last_day: Cell::new(checkin::today()),
        data_version: Cell::new(data_version),
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

    let focus = gio::SimpleAction::new("focus", None);
    let c = ctx.clone();
    focus.connect_activate(move |_, _| c.nav.push(&crate::focus::page(&c)));
    app.add_action(&focus);

    let tasks = gio::SimpleAction::new("tasks", None);
    let c = ctx.clone();
    tasks.connect_activate(move |_, _| c.nav.push(&crate::tasks_page::page(&c)));
    app.add_action(&tasks);

    let quit = gio::SimpleAction::new("quit", None);
    let a = app.clone();
    quit.connect_activate(move |_, _| a.quit());
    app.add_action(&quit);

    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("app.preferences", &["<Control>comma"]);
    app.set_accels_for_action("app.focus", &["<Control>f"]);
    app.set_accels_for_action("app.tasks", &["<Control>t"]);
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
        .timer { font-size: 64px; font-weight: 800; font-feature-settings: \"tnum\"; }
        .day-cell { min-width: 38px; min-height: 34px; padding: 0; }
        .day-cell.off { opacity: 0.5; }
        .day-cell.partial { box-shadow: inset 0 -3px 0 alpha(@accent_bg_color, 0.7); }
        .week-day { min-width: 40px; }
        .week-day .mark { font-size: 20px; }
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
