//! Seção Finanças (funções do FinanCerto com a cara do UnLoop): coluna de
//! visões (Visão geral, Lançamentos, Calendário, Contas, Cartões, Orçamento,
//! Metas, Categorias) + conteúdo do mês escolhido no cabeçalho. Também os
//! editores de lançamento, cartão e meta.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use unloop_core::checkin::{self, Datelike, NaiveDate};
use unloop_core::finance::{
    self as fin, Account, BudgetLine, Card, Category, Entry, Goal, GoalState, Kind, Pace, Repeat, Scope,
};
use unloop_core::format::format_money;
use unloop_core::model::{color, COLORS};

use crate::app::{changed_later, Shared};
use crate::widgets::{self, esc};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Overview,
    Entries,
    Calendar,
    Accounts,
    Cards,
    Budget,
    Goals,
    Categories,
}

impl View {
    const ALL: [View; 8] = [
        View::Overview,
        View::Entries,
        View::Calendar,
        View::Accounts,
        View::Cards,
        View::Budget,
        View::Goals,
        View::Categories,
    ];

    fn label(self) -> &'static str {
        match self {
            View::Overview => "Visão geral",
            View::Entries => "Lançamentos",
            View::Calendar => "Calendário",
            View::Accounts => "Contas",
            View::Cards => "Cartões",
            View::Budget => "Orçamento",
            View::Goals => "Economias",
            View::Categories => "Categorias",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            View::Overview => "view-grid-symbolic",
            View::Entries => "view-list-bullet-symbolic",
            View::Calendar => "x-office-calendar-symbolic",
            View::Accounts => "unloop-wallet-symbolic",
            View::Cards => "unloop-card-symbolic",
            View::Budget => "unloop-pie-symbolic",
            View::Goals => "unloop-target-symbolic",
            View::Categories => "view-app-grid-symbolic",
        }
    }

    /// As visões que dependem do mês mostram o seletor de mês.
    fn monthly(self) -> bool {
        !matches!(self, View::Goals | View::Categories)
    }
}

/// Filtro da lista de lançamentos.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Income,
    Expense,
    Pending,
}

struct State {
    view: Cell<View>,
    month: RefCell<String>,
    /// Dia escolhido no calendário.
    day: Cell<u32>,
    filter: Cell<Filter>,
    show_archived: Cell<bool>,
}

struct Ui {
    side: glib::WeakRef<gtk::ListBox>,
    content: glib::WeakRef<gtk::Box>,
    title: glib::WeakRef<adw::WindowTitle>,
    month_box: glib::WeakRef<gtk::Box>,
}

struct Fin {
    ctx: Shared,
    st: State,
    ui: Ui,
}

/// Nomes de categorias, contas e cartões para as linhas.
struct Lookup {
    cats: Vec<Category>,
    accounts: Vec<Account>,
    cards: Vec<Card>,
}

impl Lookup {
    fn load(ctx: &Shared) -> Self {
        Lookup {
            cats: ctx.check(ctx.db.fin_categories()).unwrap_or_default(),
            accounts: ctx.check(ctx.db.fin_accounts()).unwrap_or_default(),
            cards: ctx.check(ctx.db.fin_cards()).unwrap_or_default(),
        }
    }
    fn cat(&self, id: Option<i64>) -> Option<&Category> {
        id.and_then(|id| self.cats.iter().find(|c| c.id == id))
    }
    fn account(&self, id: Option<i64>) -> Option<&Account> {
        id.and_then(|id| self.accounts.iter().find(|a| a.id == id))
    }
    fn card(&self, id: Option<i64>) -> Option<&Card> {
        id.and_then(|id| self.cards.iter().find(|c| c.id == id))
    }
}

fn money(ctx: &Shared, value: f64) -> String {
    format_money(value, &ctx.currency())
}

fn signed_money(ctx: &Shared, kind: Kind, value: f64) -> String {
    match kind {
        Kind::Income => format!("+{}", money(ctx, value)),
        Kind::Expense => format!("−{}", money(ctx, value)),
    }
}

fn today_key() -> String {
    checkin::day_key(checkin::today())
}

// ------------------------------------------------------------------ Página

pub fn page(ctx: &Shared) -> adw::NavigationPage {
    // Coluna de visões.
    let side = gtk::ListBox::new();
    side.add_css_class("navigation-sidebar");
    side.set_selection_mode(gtk::SelectionMode::Single);
    let side_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&side)
        .build();
    let side_header = adw::HeaderBar::new();
    side_header.set_title_widget(Some(&adw::WindowTitle::new("Finanças", "")));
    let side_toolbar = adw::ToolbarView::new();
    side_toolbar.add_top_bar(&side_header);
    side_toolbar.set_content(Some(&side_scroll));

    // Conteúdo.
    let content = widgets::content_box();
    let title = adw::WindowTitle::new("", "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));
    let toggle = gtk::ToggleButton::builder().icon_name("sidebar-show-symbolic").tooltip_text("Mostrar visões").build();
    header.pack_start(&toggle);
    let add = widgets::icon_button("list-add-symbolic", "Novo lançamento (Ctrl+N)");
    header.pack_start(&add);

    let prev = widgets::icon_button("go-previous-symbolic", "Mês anterior");
    let now_button = gtk::Button::with_label("Hoje");
    now_button.add_css_class("flat");
    now_button.set_tooltip_text(Some("Voltar ao mês atual"));
    let next = widgets::icon_button("go-next-symbolic", "Próximo mês");
    let month_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    month_box.add_css_class("linked");
    month_box.append(&prev);
    month_box.append(&now_button);
    month_box.append(&next);
    header.pack_end(&month_box);

    let main_toolbar = adw::ToolbarView::new();
    main_toolbar.add_top_bar(&header);
    main_toolbar.set_content(Some(&widgets::scroller(&content)));

    let split = adw::OverlaySplitView::builder()
        .sidebar(&side_toolbar)
        .content(&main_toolbar)
        .min_sidebar_width(200.0)
        .max_sidebar_width(260.0)
        .build();
    split.bind_property("show-sidebar", &toggle, "active").bidirectional().sync_create().build();
    split.bind_property("collapsed", &toggle, "visible").sync_create().build();
    let bin = adw::BreakpointBin::builder().width_request(360).height_request(300).child(&split).build();
    if let Ok(condition) = adw::BreakpointCondition::parse("max-width: 640sp") {
        let breakpoint = adw::Breakpoint::new(condition);
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        bin.add_breakpoint(breakpoint);
    }
    let page = adw::NavigationPage::builder().title("Finanças").child(&bin).build();

    let f = Rc::new(Fin {
        ctx: ctx.clone(),
        st: State {
            view: Cell::new(View::Overview),
            month: RefCell::new(fin::current_month()),
            day: Cell::new(checkin::today().day()),
            filter: Cell::new(Filter::All),
            show_archived: Cell::new(false),
        },
        ui: Ui {
            side: side.downgrade(),
            content: content.downgrade(),
            title: title.downgrade(),
            month_box: month_box.downgrade(),
        },
    });

    let (fc, ws) = (f.clone(), split.downgrade());
    side.connect_row_activated(move |_, row| {
        let Some(view) = View::ALL.get(row.index().max(0) as usize).copied() else { return };
        fc.st.view.set(view);
        if let Some(split) = ws.upgrade() {
            if split.is_collapsed() {
                split.set_show_sidebar(false);
            }
        }
        rerender(&fc);
    });

    let fc = f.clone();
    prev.connect_clicked(move |_| shift_month(&fc, -1));
    let fc = f.clone();
    next.connect_clicked(move |_| shift_month(&fc, 1));
    let fc = f.clone();
    now_button.connect_clicked(move |_| {
        *fc.st.month.borrow_mut() = fin::current_month();
        fc.st.day.set(checkin::today().day());
        rerender(&fc);
    });
    let fc = f.clone();
    add.connect_clicked(move |_| new_entry(&fc));

    // Ctrl+N: novo lançamento (enquanto a seção estiver aberta).
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_scope(gtk::ShortcutScope::Managed);
    let fc = f.clone();
    shortcuts.add_shortcut(gtk::Shortcut::new(
        gtk::ShortcutTrigger::parse_string("<Control>n"),
        Some(gtk::CallbackAction::new(move |_, _| {
            new_entry(&fc);
            glib::Propagation::Stop
        })),
    ));
    bin.add_controller(shortcuts);

    render(&f);
    let fc = f.clone();
    ctx.on_refresh(move || render(&fc));
    page
}

fn new_entry(f: &Rc<Fin>) {
    let month = f.st.month.borrow().clone();
    let mut e = Entry::new(Kind::Expense, "", 0.0, &month);
    if f.st.view.get() == View::Calendar {
        e.due_date = Some(fin::day_in_month(&month, f.st.day.get()));
    } else if month == fin::current_month() {
        e.due_date = Some(today_key());
    }
    if f.st.filter.get() == Filter::Income && f.st.view.get() == View::Entries {
        e.kind = Kind::Income;
    }
    f.ctx.nav.push(&entry_editor(&f.ctx, e));
}

fn shift_month(f: &Rc<Fin>, delta: i32) {
    let next = fin::add_months(&f.st.month.borrow(), delta);
    f.st.day.set(f.st.day.get().min(fin::days_in_month(&next)));
    *f.st.month.borrow_mut() = next;
    rerender(f);
}

/// Reconstrói depois do sinal atual (o widget que emitiu será removido).
fn rerender(f: &Rc<Fin>) {
    let fc = f.clone();
    glib::idle_add_local_once(move || {
        render(&fc);
    });
}

fn side_row(view: View, count: Option<usize>) -> gtk::ListBoxRow {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    b.append(&gtk::Image::from_icon_name(view.icon()));
    let label = gtk::Label::new(Some(view.label()));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    b.append(&label);
    if let Some(n) = count.filter(|n| *n > 0) {
        let count = gtk::Label::new(Some(&n.to_string()));
        count.add_css_class("sidebar-count");
        b.append(&count);
    }
    gtk::ListBoxRow::builder().child(&b).build()
}

fn render(f: &Rc<Fin>) -> bool {
    let (Some(side), Some(content), Some(title), Some(month_box)) =
        (f.ui.side.upgrade(), f.ui.content.upgrade(), f.ui.title.upgrade(), f.ui.month_box.upgrade())
    else {
        return false;
    };
    let ctx = &f.ctx;
    let month = f.st.month.borrow().clone();
    let view = f.st.view.get();
    let today = checkin::today();

    // Contadores da coluna: pendências do mês e metas ativas.
    let entries = ctx.check(ctx.db.fin_entries(&month)).unwrap_or_default();
    let overdue = ctx.check(ctx.db.fin_overdue_before(&month)).unwrap_or_default();
    let pending = entries.iter().filter(|e| !e.paid && e.kind == Kind::Expense).count() + overdue.len();
    let late = entries.iter().chain(&overdue).filter(|e| e.is_overdue(today)).count();
    let goals = ctx.check(ctx.db.fin_goals()).unwrap_or_default();
    let active_goals = goals.iter().filter(|g| g.state == GoalState::Active).count();

    side.remove_all();
    for v in View::ALL {
        let count = match v {
            View::Entries => Some(pending),
            View::Calendar => Some(late),
            View::Goals => Some(active_goals),
            _ => None,
        };
        side.append(&side_row(v, count));
    }
    if let Some(i) = View::ALL.iter().position(|v| *v == view) {
        side.select_row(side.row_at_index(i as i32).as_ref());
    }

    title.set_title(view.label());
    title.set_subtitle(&if view.monthly() { fin::month_label(&month) } else { String::new() });
    month_box.set_visible(view.monthly());

    widgets::clear_box(&content);
    match view {
        View::Overview => fill_overview(f, &content, &month),
        View::Entries => fill_entries(f, &content, &month, entries, overdue),
        View::Calendar => fill_calendar(f, &content, &month, &entries),
        View::Accounts => fill_accounts(f, &content, &month),
        View::Cards => fill_cards(f, &content, &month),
        View::Budget => fill_budget(f, &content, &month),
        View::Goals => fill_goals(f, &content, &goals),
        View::Categories => fill_categories(f, &content),
    }
    true
}

fn empty(b: &gtk::Box, icon: &str, title: &str, description: &str, child: Option<&gtk::Widget>) {
    let status = adw::StatusPage::builder().icon_name(icon).title(title).description(description).build();
    if let Some(c) = child {
        status.set_child(Some(c));
    }
    b.append(&status);
}

fn suffix_button(label: &str, icon: &str) -> gtk::Button {
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    inner.append(&gtk::Image::from_icon_name(icon));
    inner.append(&gtk::Label::new(Some(label)));
    let b = gtk::Button::builder().child(&inner).valign(gtk::Align::Center).build();
    b.add_css_class("flat");
    b
}

fn header_buttons(buttons: &[&gtk::Button]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    for button in buttons {
        b.append(*button);
    }
    b
}

/// Linha clicável com ícone (substitui `adw::ButtonRow`, que pede libadwaita 1.6).
fn button_row(title: &str, icon: Option<&str>) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).activatable(true).build();
    if let Some(i) = icon {
        row.add_prefix(&gtk::Image::from_icon_name(i));
    }
    row.add_css_class("accent");
    row
}

fn dot(color_code: i64) -> gtk::Box {
    let d = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    d.add_css_class("list-dot");
    d.add_css_class(&format!("habit-color-{}", color(color_code).code));
    d.set_valign(gtk::Align::Center);
    d.set_halign(gtk::Align::Center);
    d
}

fn amount_label(text: &str, class: Option<&str>) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("fin-amount");
    if let Some(c) = class {
        l.add_css_class(c);
    }
    l.set_valign(gtk::Align::Center);
    l
}

fn emoji_avatar(emoji: &str, color_code: i64) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("avatar");
    b.add_css_class(&format!("habit-color-{}", color(color_code).code));
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::Start);
    b.set_hexpand(false);
    let l = gtk::Label::new(Some(emoji));
    l.set_vexpand(true);
    b.append(&l);
    b
}

// ------------------------------------------------------------ Visão geral

fn fill_overview(f: &Rc<Fin>, b: &gtk::Box, month: &str) {
    let ctx = &f.ctx;
    let Some(o) = ctx.check(ctx.db.fin_overview(month)) else { return };
    let look = Lookup::load(ctx);
    let t = o.totals;

    if look.accounts.is_empty() && t.income_total == 0.0 && t.expense_total == 0.0 && look.cards.is_empty() {
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        buttons.set_halign(gtk::Align::Center);
        let account = widgets::pill_button("Criar uma conta", Some("suggested-action"));
        let entry = widgets::pill_button("Novo lançamento", None);
        buttons.append(&account);
        buttons.append(&entry);
        let c = ctx.clone();
        account.connect_clicked(move |_| account_dialog(&c, None));
        let fc = f.clone();
        entry.connect_clicked(move |_| new_entry(&fc));
        empty(
            b,
            "unloop-wallet-symbolic",
            "Suas finanças, só suas",
            "Cadastre as contas (banco, carteira) e lance receitas e despesas. Nada sai deste computador.",
            Some(buttons.upcast_ref()),
        );
        return;
    }

    let stats = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(3)
        .column_spacing(12)
        .row_spacing(12)
        .build();
    let card = |value: f64, caption: &str, icon: &str, colored: bool| {
        let c = crate::home::stat_card(&money(ctx, value), caption, icon);
        if colored {
            if let Some(label) = c.first_child().and_then(|w| w.next_sibling()) {
                label.add_css_class(if value < 0.0 { "error" } else { "success" });
            }
        }
        c
    };
    stats.append(&card(o.accounts_total, "saldo nas contas", "unloop-wallet-symbolic", true));
    stats.append(&card(
        t.income_total,
        &format!("receitas · {} recebido", money(ctx, t.income_received)),
        "go-down-symbolic",
        false,
    ));
    stats.append(&card(
        t.expense_total,
        &format!("despesas · {} pago", money(ctx, t.expense_paid)),
        "go-up-symbolic",
        false,
    ));
    stats.append(&card(t.available(), "sobra do mês (receitas − despesas)", "unloop-pie-symbolic", true));
    stats.append(&card(o.forecast_end, "previsão no fim do mês", "x-office-calendar-symbolic", true));
    let daily_caption = if o.days_left > 0 {
        format!("por dia nos {} dias restantes", o.days_left)
    } else {
        "mês encerrado".to_string()
    };
    stats.append(&card(o.daily_allowance, &daily_caption, "alarm-symbolic", false));
    b.append(&stats);

    // Comparação com o mês anterior.
    if o.previous.expense_total > 0.0 {
        let diff = t.expense_total - o.previous.expense_total;
        let pct = diff / o.previous.expense_total * 100.0;
        let text = if diff.abs() < 0.005 {
            "Despesas iguais às do mês anterior.".to_string()
        } else {
            format!(
                "Despesas {} {:.0}% em relação a {} ({}).",
                if diff > 0.0 { "subiram" } else { "caíram" },
                pct.abs(),
                fin::month_short(&fin::add_months(month, -1)),
                money(ctx, o.previous.expense_total)
            )
        };
        b.append(&widgets::label(&text, &["dim-label"]));
    }

    // Contas a pagar.
    let late = o.due_soon.iter().filter(|e| e.is_overdue(checkin::today())).count();
    let summary = if o.due_soon.is_empty() {
        "Nada vencendo nos próximos 7 dias.".to_string()
    } else if late > 0 {
        format!("{late} atrasada(s) · {} no total", money(ctx, o.due_soon.iter().map(|e| e.amount).sum()))
    } else {
        format!("{} nos próximos 7 dias", money(ctx, o.due_soon.iter().map(|e| e.amount).sum()))
    };
    let group = widgets::group("A pagar", Some(summary.as_str()));
    if !o.due_soon.is_empty() {
        let list = widgets::boxed_list();
        for e in &o.due_soon {
            list.append(&entry_row(ctx, e, &look, e.month != month));
        }
        group.add(&list);
    }
    b.append(&group);

    // Gastos por categoria.
    if !o.by_category.is_empty() {
        let total: f64 = o.by_category.iter().map(|c| c.1).sum();
        let group = widgets::group("Gastos por categoria", Some(&format!("{} no mês", money(ctx, total))));
        let row = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(1)
            .max_children_per_line(2)
            .column_spacing(24)
            .row_spacing(12)
            .build();
        let slices: Vec<(f64, String)> = o
            .by_category
            .iter()
            .map(|(c, v)| (*v, color(c.as_ref().map_or(1021, |c| c.color)).hex.to_string()))
            .collect();
        let donut = widgets::donut_chart(slices, &money(ctx, total), 180);
        donut.set_valign(gtk::Align::Start);
        row.append(&donut);
        let legend = widgets::boxed_list();
        legend.set_hexpand(true);
        legend.set_size_request(280, -1);
        legend.set_valign(gtk::Align::Start);
        for (cat, value) in o.by_category.iter().take(8) {
            let name = cat.as_ref().map_or("Sem categoria", |c| c.name.as_str());
            let r = adw::ActionRow::builder()
                .title(esc(name))
                .subtitle(format!("{:.0}%", value / total * 100.0))
                .build();
            r.add_prefix(&dot(cat.as_ref().map_or(1021, |c| c.color)));
            r.add_suffix(&amount_label(&money(ctx, *value), None));
            legend.append(&r);
        }
        row.append(&legend);
        group.add(&row);
        b.append(&group);
    }

    // Tendência.
    if let Some(trend) = ctx.check(ctx.db.fin_trend(month, 6)) {
        if trend.iter().any(|p| p.income > 0.0 || p.expense > 0.0) {
            let group = widgets::group("Últimos 6 meses", Some("Receitas (verde) e despesas (vermelho)."));
            group.add(&trend_chart(ctx, &trend));
            b.append(&group);
        }
    }

    // Contas.
    if !o.accounts.is_empty() {
        let group = widgets::group("Contas", Some(&format!("Total: {}", money(ctx, o.accounts_total))));
        let list = widgets::boxed_list();
        for a in &o.accounts {
            list.append(&account_row(ctx, a));
        }
        group.add(&list);
        b.append(&group);
    }

    // Cartões.
    let cards = ctx.check(ctx.db.fin_card_summaries(month)).unwrap_or_default();
    if !cards.is_empty() {
        let group = widgets::group("Cartões", None);
        let list = widgets::boxed_list();
        for s in &cards {
            let r = adw::ActionRow::builder()
                .title(esc(&s.card.name))
                .subtitle(card_subtitle(s))
                .activatable(true)
                .build();
            r.add_prefix(&dot(s.card.color));
            r.add_suffix(&amount_label(&money(ctx, s.invoice), if s.invoice_paid { Some("dim-label") } else { None }));
            let (c, id, m) = (ctx.clone(), s.card.id, month.to_string());
            r.connect_activated(move |_| c.nav.push(&card_page(&c, id, &m)));
            list.append(&r);
        }
        group.add(&list);
        b.append(&group);
    }

    // Parcelas futuras.
    if !o.future_installments.is_empty() {
        let group = widgets::group("Próximas parcelas", None);
        let list = widgets::boxed_list();
        for e in &o.future_installments {
            list.append(&entry_row(ctx, e, &look, true));
        }
        group.add(&list);
        b.append(&group);
    }
}

/// Barras agrupadas (receita × despesa) por mês.
fn trend_chart(ctx: &Shared, points: &[fin::TrendPoint]) -> gtk::Box {
    let chart = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    chart.set_homogeneous(true);
    chart.set_height_request(170);
    chart.set_margin_top(6);
    let max = points.iter().map(|p| p.income.max(p.expense)).fold(0.0, f64::max).max(1.0);
    for p in points {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let bars = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        bars.set_halign(gtk::Align::Center);
        bars.set_vexpand(true);
        for (value, class) in [(p.income, "bar-color-1002"), (p.expense, "bar-color-1001")] {
            let bar = gtk::ProgressBar::new();
            bar.set_orientation(gtk::Orientation::Vertical);
            bar.set_inverted(true);
            bar.set_vexpand(true);
            bar.set_fraction(value / max);
            bar.add_css_class("thick");
            bar.add_css_class(class);
            bars.append(&bar);
        }
        column.append(&bars);
        column.set_tooltip_text(Some(&format!(
            "{}\nReceitas: {}\nDespesas: {}\nSaldo: {}",
            fin::month_label(&p.month),
            money(ctx, p.income),
            money(ctx, p.expense),
            money(ctx, p.income - p.expense)
        )));
        let caption = gtk::Label::new(Some(&fin::month_short(&p.month)));
        caption.add_css_class("caption");
        caption.add_css_class("dim-label");
        column.append(&caption);
        chart.append(&column);
    }
    chart
}

// ------------------------------------------------------------ Lançamentos

fn entry_subtitle(e: &Entry, look: &Lookup, show_month: bool) -> String {
    let mut parts = Vec::new();
    let today = checkin::today();
    if let Some(date) = e.effective_date() {
        let mark = if e.is_overdue(today) { "⚠️" } else { "📅" };
        parts.push(format!("{mark} {}", fin::date_short(date)));
    }
    if show_month {
        parts.push(fin::month_short(&e.month));
    }
    if e.is_transfer() {
        parts.push("↔ transferência".into());
    } else if let Some(card) = look.card(e.card_id) {
        parts.push(format!("💳 {}", card.name));
    } else if let Some(cat) = look.cat(e.category_id) {
        parts.push(cat.name.clone());
    }
    if let Some(acc) = look.account(e.account_id) {
        parts.push(acc.name.clone());
    }
    if e.is_recurring() {
        parts.push("🔁".into());
    }
    if e.excluded {
        parts.push("fora dos totais".into());
    }
    if !e.note.is_empty() {
        parts.push("📝".into());
    }
    parts.join(" · ")
}

/// Linha de lançamento: caixa de pago, descrição, detalhes e valor.
fn entry_row(ctx: &Shared, e: &Entry, look: &Lookup, show_month: bool) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(esc(&e.description))
        .subtitle(esc(&entry_subtitle(e, look, show_month)))
        .activatable(true)
        .build();
    let check = gtk::CheckButton::new();
    check.set_active(e.paid);
    check.set_valign(gtk::Align::Center);
    check.set_tooltip_text(Some(match e.kind {
        Kind::Income => "Recebido",
        Kind::Expense => "Pago",
    }));
    if e.is_transfer() {
        check.set_sensitive(false);
    }
    let (c, id) = (ctx.clone(), e.id);
    check.connect_toggled(move |cb| {
        if c.check(c.db.fin_set_paid(id, cb.is_active(), None)).is_some() {
            changed_later(&c);
        }
    });
    row.add_prefix(&check);
    let class = if e.is_transfer() || e.excluded {
        "dim-label"
    } else if e.kind == Kind::Income {
        "success"
    } else if e.is_overdue(checkin::today()) {
        "error"
    } else {
        "fin-expense"
    };
    row.add_suffix(&amount_label(&signed_money(ctx, e.kind, e.amount), Some(class)));
    if e.paid {
        row.add_css_class("fin-paid");
    }
    let (c, entry) = (ctx.clone(), e.clone());
    row.connect_activated(move |_| {
        if let Some(card) = entry.card_id {
            c.nav.push(&card_page(&c, card, &entry.month));
        } else if let Some(fresh) = c.check(c.db.fin_entry(entry.id)).flatten() {
            c.nav.push(&entry_editor(&c, fresh));
        }
    });
    row
}

fn filter_bar(f: &Rc<Fin>) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.add_css_class("linked");
    bar.set_halign(gtk::Align::Center);
    let options = [
        (Filter::All, "Todos"),
        (Filter::Income, "Receitas"),
        (Filter::Expense, "Despesas"),
        (Filter::Pending, "Pendentes"),
    ];
    let mut first: Option<gtk::ToggleButton> = None;
    for (filter, label) in options {
        let b = gtk::ToggleButton::with_label(label);
        if let Some(group) = &first {
            b.set_group(Some(group));
        } else {
            first = Some(b.clone());
        }
        b.set_active(f.st.filter.get() == filter);
        let fc = f.clone();
        b.connect_toggled(move |b| {
            if b.is_active() && fc.st.filter.get() != filter {
                fc.st.filter.set(filter);
                rerender(&fc);
            }
        });
        bar.append(&b);
    }
    bar
}

fn fill_entries(f: &Rc<Fin>, b: &gtk::Box, month: &str, entries: Vec<Entry>, overdue: Vec<Entry>) {
    let ctx = &f.ctx;
    let look = Lookup::load(ctx);
    let totals = fin::MonthTotals::from_entries(&entries);

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    top.append(&filter_bar(f));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    top.append(&spacer);
    let transfer = suffix_button("Transferir", "media-playlist-repeat-symbolic");
    let c = ctx.clone();
    let m = month.to_string();
    transfer.connect_clicked(move |_| transfer_dialog(&c, &m));
    top.append(&transfer);
    b.append(&top);

    let filter = f.st.filter.get();
    let keep = |e: &&Entry| match filter {
        Filter::All => true,
        Filter::Income => e.kind == Kind::Income,
        Filter::Expense => e.kind == Kind::Expense,
        Filter::Pending => !e.paid,
    };

    let list_group = |title: &str, description: String, items: Vec<&Entry>, show_month: bool| {
        if items.is_empty() {
            return;
        }
        let group = widgets::group(title, Some(description.as_str()));
        let list = widgets::boxed_list();
        for e in items {
            list.append(&entry_row(ctx, e, &look, show_month));
        }
        group.add(&list);
        b.append(&group);
    };

    let late: Vec<&Entry> = overdue.iter().filter(keep).collect();
    let late_total: f64 = late.iter().map(|e| e.amount).sum();
    list_group("⚠️ Em aberto de meses anteriores", money(ctx, late_total), late, true);

    let income: Vec<&Entry> = entries.iter().filter(|e| e.kind == Kind::Income).filter(keep).collect();
    list_group(
        "Receitas",
        format!("{} · {} recebido", money(ctx, totals.income_total), money(ctx, totals.income_received)),
        income,
        false,
    );
    let expense: Vec<&Entry> = entries.iter().filter(|e| e.kind == Kind::Expense).filter(keep).collect();
    list_group(
        "Despesas",
        format!("{} · {} pago", money(ctx, totals.expense_total), money(ctx, totals.expense_paid)),
        expense,
        false,
    );

    if entries.iter().filter(keep).count() == 0 && overdue.iter().filter(keep).count() == 0 {
        let add = widgets::pill_button("Novo lançamento", Some("suggested-action"));
        add.set_halign(gtk::Align::Center);
        let fc = f.clone();
        add.connect_clicked(move |_| new_entry(&fc));
        empty(
            b,
            "view-list-bullet-symbolic",
            "Nada por aqui",
            &format!("Sem lançamentos em {} com esse filtro.", fin::month_label(month)),
            Some(add.upcast_ref()),
        );
    } else {
        let saldo = totals.available();
        let label = widgets::label(
            &format!("Receitas − despesas do mês: {}", money(ctx, saldo)),
            &["dim-label", if saldo < 0.0 { "error" } else { "success" }],
        );
        b.append(&label);
    }
}

// --------------------------------------------------------------- Calendário

fn fill_calendar(f: &Rc<Fin>, b: &gtk::Box, month: &str, entries: &[Entry]) {
    let ctx = &f.ctx;
    let look = Lookup::load(ctx);
    let today = checkin::today();
    let first = fin::first_day(month);
    let days = fin::days_in_month(month);
    let selected = f.st.day.get().clamp(1, days);
    let day_of = |e: &Entry| e.effective_date().and_then(fin::parse_date).filter(|d| fin::month_of(*d) == month).map(|d| d.day());

    let grid = gtk::Grid::builder().column_homogeneous(true).row_spacing(4).column_spacing(4).build();
    grid.set_halign(gtk::Align::Fill);
    for (i, name) in checkin::WEEKDAY_SHORT.iter().enumerate() {
        let l = gtk::Label::new(Some(name));
        l.add_css_class("caption-heading");
        l.add_css_class("dim-label");
        grid.attach(&l, i as i32, 0, 1, 1);
    }
    let offset = first.weekday().num_days_from_monday() as i32;
    for day in 1..=days {
        let pos = offset + day as i32 - 1;
        let items: Vec<&Entry> = entries.iter().filter(|e| day_of(e) == Some(day)).collect();
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let number = gtk::Label::new(Some(&day.to_string()));
        number.add_css_class("heading");
        cell.append(&number);
        let marks = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        marks.set_halign(gtk::Align::Center);
        let date = NaiveDate::from_ymd_opt(first.year(), first.month(), day).unwrap_or(first);
        let has = |pred: &dyn Fn(&Entry) -> bool| items.iter().any(|e| pred(e));
        if has(&|e| e.kind == Kind::Income && !e.is_transfer()) {
            marks.append(&mark("success"));
        }
        if has(&|e| e.kind == Kind::Expense && !e.paid) {
            marks.append(&mark(if date < today { "error" } else { "warning" }));
        }
        if has(&|e| e.kind == Kind::Expense && e.paid && !e.is_transfer()) {
            marks.append(&mark("dim-label"));
        }
        cell.append(&marks);
        let button = gtk::Button::builder().child(&cell).build();
        button.add_css_class("flat");
        button.add_css_class("fin-day");
        if date == today {
            button.add_css_class("today");
        }
        if day == selected {
            button.add_css_class("selected");
        }
        if !items.is_empty() {
            let total: f64 = items.iter().filter(|e| e.counts()).map(|e| e.kind.sign() * e.amount).sum();
            button.set_tooltip_text(Some(&format!("{} lançamento(s) · {}", items.len(), money(ctx, total))));
        }
        let fc = f.clone();
        button.connect_clicked(move |_| {
            fc.st.day.set(day);
            rerender(&fc);
        });
        grid.attach(&button, pos % 7, 1 + pos / 7, 1, 1);
    }
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("card");
    frame.add_css_class("fin-calendar");
    frame.append(&grid);
    b.append(&frame);

    let legend = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    legend.set_halign(gtk::Align::Center);
    for (class, text) in [("success", "receita"), ("warning", "a pagar"), ("error", "atrasada"), ("dim-label", "paga")] {
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        item.append(&mark(class));
        let l = gtk::Label::new(Some(text));
        l.add_css_class("caption");
        l.add_css_class("dim-label");
        item.append(&l);
        legend.append(&item);
    }
    b.append(&legend);

    let date = fin::day_in_month(month, selected);
    let items: Vec<&Entry> = entries.iter().filter(|e| day_of(e) == Some(selected)).collect();
    let weekday = checkin::WEEKDAY_SHORT
        [fin::parse_date(&date).map_or(0, |d| d.weekday().num_days_from_monday() as usize)];
    let group = widgets::group(
        &format!("{weekday}, {}", fin::date_short(&date)),
        Some(if items.is_empty() { "Nenhum lançamento neste dia." } else { "" }).filter(|s| !s.is_empty()),
    );
    let add = suffix_button("Lançar neste dia", "list-add-symbolic");
    let (c, m, d) = (ctx.clone(), month.to_string(), date.clone());
    add.connect_clicked(move |_| {
        let mut e = Entry::new(Kind::Expense, "", 0.0, &m);
        e.due_date = Some(d.clone());
        c.nav.push(&entry_editor(&c, e));
    });
    group.set_header_suffix(Some(&add));
    if !items.is_empty() {
        let list = widgets::boxed_list();
        for e in items {
            list.append(&entry_row(ctx, e, &look, false));
        }
        group.add(&list);
    }
    b.append(&group);

    let undated: Vec<&Entry> = entries.iter().filter(|e| day_of(e).is_none()).collect();
    if !undated.is_empty() {
        let group = widgets::group("Sem data", None);
        let list = widgets::boxed_list();
        for e in undated {
            list.append(&entry_row(ctx, e, &look, false));
        }
        group.add(&list);
        b.append(&group);
    }
}

fn mark(class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some("●"));
    l.add_css_class("fin-mark");
    l.add_css_class(class);
    l
}

// ------------------------------------------------------------------ Contas

fn account_row(ctx: &Shared, a: &fin::AccountBalance) -> adw::ActionRow {
    let mut parts = vec![format!("Previsto: {}", money(ctx, a.forecast))];
    if a.month_income > 0.0 || a.month_expense > 0.0 {
        parts.push(format!("no mês +{} / −{}", money(ctx, a.month_income), money(ctx, a.month_expense)));
    }
    let row = adw::ActionRow::builder()
        .title(esc(&a.account.name))
        .subtitle(esc(&parts.join(" · ")))
        .activatable(true)
        .build();
    row.add_prefix(&dot(a.account.color));
    row.add_suffix(&amount_label(&money(ctx, a.current), Some(if a.current < 0.0 { "error" } else { "fin-balance" })));
    let (c, acc) = (ctx.clone(), a.account.clone());
    row.connect_activated(move |_| account_dialog(&c, Some(acc.clone())));
    row
}

fn fill_accounts(f: &Rc<Fin>, b: &gtk::Box, month: &str) {
    let ctx = &f.ctx;
    let balances = ctx.check(ctx.db.fin_account_balances(month)).unwrap_or_default();
    let new_account = suffix_button("Nova conta", "list-add-symbolic");
    let c = ctx.clone();
    new_account.connect_clicked(move |_| account_dialog(&c, None));
    let transfer = suffix_button("Transferir", "media-playlist-repeat-symbolic");
    let (c, m) = (ctx.clone(), month.to_string());
    transfer.connect_clicked(move |_| transfer_dialog(&c, &m));
    transfer.set_sensitive(balances.len() >= 2);

    let total: f64 = balances.iter().map(|a| a.current).sum();
    let forecast: f64 = balances.iter().map(|a| a.forecast).sum();
    let description = format!("Saldo total {} · previsto até o fim do mês {}", money(ctx, total), money(ctx, forecast));
    let group = widgets::group("Contas", Some(description.as_str()));
    group.set_header_suffix(Some(&header_buttons(&[&transfer, &new_account])));
    if balances.is_empty() {
        b.append(&group);
        empty(
            b,
            "unloop-wallet-symbolic",
            "Nenhuma conta",
            "Crie uma conta para cada lugar onde fica o seu dinheiro: banco, carteira, poupança. O saldo é calculado pelos lançamentos pagos.",
            None,
        );
        return;
    }
    let list = widgets::boxed_list();
    for a in &balances {
        list.append(&account_row(ctx, a));
    }
    group.add(&list);
    b.append(&group);
    b.append(&widgets::label(
        "Saldo = saldo inicial + entradas recebidas − saídas pagas. Toque numa conta para editar.",
        &["caption", "dim-label"],
    ));
}

fn color_combo(title: &str, current: i64) -> adw::ComboRow {
    let labels: Vec<&str> = COLORS.iter().map(|c| c.label).collect();
    let row = adw::ComboRow::builder().title(title).model(&gtk::StringList::new(&labels)).build();
    row.set_selected(COLORS.iter().position(|c| c.code == current).unwrap_or(0) as u32);
    row
}

fn color_from(row: &adw::ComboRow) -> i64 {
    COLORS.get(row.selected() as usize).map_or(1003, |c| c.code)
}

fn money_row(title: &str, value: f64, allow_negative: bool) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(if allow_negative { -1e10 } else { 0.0 }, 1e10, 1.0);
    row.set_title(title);
    row.set_digits(2);
    row.set_value(value);
    row
}

/// Diálogo com um formulário em lista. `on_ok` devolve `false` para manter os dados (erro).
fn form_dialog(
    ctx: &Shared,
    heading: &str,
    body: &str,
    form: &gtk::ListBox,
    extra: &[(&str, &str, adw::ResponseAppearance)],
    on_response: impl Fn(&str) + 'static,
) {
    form.set_size_request(380, -1);
    let d = adw::MessageDialog::new(Some(&ctx.window), Some(heading), if body.is_empty() { None } else { Some(body) });
    d.set_extra_child(Some(form));
    d.add_response("cancel", "Cancelar");
    for (id, label, appearance) in extra {
        d.add_response(id, label);
        d.set_response_appearance(id, *appearance);
    }
    d.set_default_response(extra.last().map(|e| e.0));
    d.set_close_response("cancel");
    d.connect_response(None, move |_, response| {
        if response != "cancel" {
            on_response(response);
        }
    });
    d.present();
}

fn account_dialog(ctx: &Shared, account: Option<Account>) {
    let editing = account.is_some();
    let a = account.unwrap_or(Account { id: 0, name: String::new(), initial_balance: 0.0, color: 1003, position: 0 });
    let form = widgets::boxed_list();
    let name = adw::EntryRow::builder().title("Nome (ex.: Nubank, Carteira)").build();
    name.set_text(&a.name);
    let initial = money_row("Saldo inicial", a.initial_balance, true);
    let color_row = color_combo("Cor", a.color);
    form.append(&name);
    form.append(&initial);
    form.append(&color_row);
    let mut actions = vec![("save", "Salvar", adw::ResponseAppearance::Suggested)];
    if editing {
        actions.insert(0, ("delete", "Apagar", adw::ResponseAppearance::Destructive));
    }
    let c = ctx.clone();
    form_dialog(
        ctx,
        if editing { "Editar conta" } else { "Nova conta" },
        "O saldo inicial é o valor que havia na conta antes do primeiro lançamento.",
        &form,
        &actions,
        move |response| {
            if response == "delete" {
                let c2 = c.clone();
                widgets::confirm(
                    &c,
                    "Apagar conta?",
                    "Os lançamentos continuam, só que sem conta.",
                    "Apagar",
                    true,
                    move || {
                        if c2.check(c2.db.fin_delete_account(a.id)).is_some() {
                            c2.changed();
                        }
                    },
                );
                return;
            }
            let n = name.text().trim().to_string();
            if n.is_empty() {
                c.toast("Dê um nome à conta.");
                return;
            }
            let ok = if editing {
                let updated = Account { name: n, initial_balance: initial.value(), color: color_from(&color_row), ..a.clone() };
                c.check(c.db.fin_update_account(&updated)).is_some()
            } else {
                c.check(c.db.fin_insert_account(&n, initial.value(), color_from(&color_row))).is_some()
            };
            if ok {
                c.changed();
            }
        },
    );
}

fn transfer_dialog(ctx: &Shared, month: &str) {
    let accounts = ctx.check(ctx.db.fin_accounts()).unwrap_or_default();
    if accounts.len() < 2 {
        widgets::info(ctx, "Transferência", "Crie pelo menos duas contas para transferir entre elas.");
        return;
    }
    let names: Vec<&str> = accounts.iter().map(|a| a.name.as_str()).collect();
    let form = widgets::boxed_list();
    let from = adw::ComboRow::builder().title("De").model(&gtk::StringList::new(&names)).build();
    let to = adw::ComboRow::builder().title("Para").model(&gtk::StringList::new(&names)).build();
    to.set_selected(1);
    let amount = money_row("Valor", 0.0, false);
    let initial = if month == fin::current_month() { today_key() } else { fin::day_in_month(month, 1) };
    let (date_row, date) = date_row("Data", Some(initial), false);
    let description = adw::EntryRow::builder().title("Descrição (opcional)").build();
    form.append(&from);
    form.append(&to);
    form.append(&amount);
    form.append(&date_row);
    form.append(&description);
    let c = ctx.clone();
    form_dialog(
        ctx,
        "Transferir entre contas",
        "Sai de uma conta e entra na outra. Não conta como receita nem despesa.",
        &form,
        &[("ok", "Transferir", adw::ResponseAppearance::Suggested)],
        move |_| {
            let (Some(a), Some(b)) = (accounts.get(from.selected() as usize), accounts.get(to.selected() as usize)) else {
                return;
            };
            if a.id == b.id {
                c.toast("Escolha contas diferentes.");
                return;
            }
            if amount.value() <= 0.0 {
                c.toast("Informe um valor maior que zero.");
                return;
            }
            let d = date.borrow().clone().unwrap_or_else(today_key);
            if c.check(c.db.fin_transfer(a.id, b.id, amount.value(), &d, &description.text())).is_some() {
                c.toast("Transferência registrada.");
                c.changed();
            }
        },
    );
}

// ----------------------------------------------------------------- Cartões

fn card_subtitle(s: &fin::CardSummary) -> String {
    let mut parts = vec![format!(
        "Fatura {}",
        if s.invoice_paid {
            "paga".to_string()
        } else {
            format!("vence dia {}", s.card.due_day)
        }
    )];
    if s.card.credit_limit > 0.0 {
        parts.push(format!("{:.0}% do limite usado", s.usage() * 100.0));
    }
    parts.join(" · ")
}

fn fill_cards(f: &Rc<Fin>, b: &gtk::Box, month: &str) {
    let ctx = &f.ctx;
    let summaries = ctx.check(ctx.db.fin_card_summaries(month)).unwrap_or_default();
    let look = Lookup::load(ctx);
    let new_card = suffix_button("Novo cartão", "list-add-symbolic");
    let c = ctx.clone();
    new_card.connect_clicked(move |_| c.nav.push(&card_editor(&c, None)));
    let top = widgets::group(
        "Cartões de crédito",
        Some("Cada compra cai na fatura certa pelo dia de fechamento. A fatura vira uma despesa do mês."),
    );
    top.set_header_suffix(Some(&new_card));
    b.append(&top);
    if summaries.is_empty() {
        empty(b, "unloop-card-symbolic", "Nenhum cartão", "Cadastre um cartão para lançar as compras e acompanhar a fatura.", None);
        return;
    }
    for s in &summaries {
        let group = widgets::group(
            &s.card.name,
            Some(&format!(
                "Fecha dia {} · vence dia {}{}",
                s.card.closing_day,
                s.card.due_day,
                look.account(s.card.account_id).map_or(String::new(), |a| format!(" · paga pela conta {}", a.name))
            )),
        );
        let edit = widgets::icon_button("document-edit-symbolic", "Editar cartão");
        let (c, card) = (ctx.clone(), s.card.clone());
        edit.connect_clicked(move |_| c.nav.push(&card_editor(&c, Some(card.clone()))));
        group.set_header_suffix(Some(&edit));
        let list = widgets::boxed_list();

        let invoice = adw::ActionRow::builder()
            .title(format!("Fatura de {}", fin::month_label(month)))
            .subtitle(if s.invoice_paid {
                "Paga ✓".to_string()
            } else if s.invoice > 0.0 {
                format!("Vence {}", fin::date_short(&fin::day_in_month(month, s.card.due_day)))
            } else {
                "Sem compras nesta fatura".to_string()
            })
            .activatable(true)
            .build();
        invoice.add_prefix(&dot(s.card.color));
        invoice.add_suffix(&amount_label(&money(ctx, s.invoice), if s.invoice_paid { Some("dim-label") } else { None }));
        if s.invoice > 0.0 {
            let pay = gtk::Button::with_label(if s.invoice_paid { "Desfazer" } else { "Pagar" });
            pay.set_valign(gtk::Align::Center);
            if !s.invoice_paid {
                pay.add_css_class("suggested-action");
            }
            let (c, id, m, paid) = (ctx.clone(), s.card.id, month.to_string(), s.invoice_paid);
            pay.connect_clicked(move |_| {
                if let Some(e) = c.check(c.db.fin_invoice_entry(id, &m)).flatten() {
                    if c.check(c.db.fin_set_paid(e.id, !paid, None)).is_some() {
                        if !paid {
                            c.toast("Fatura marcada como paga.");
                        }
                        changed_later(&c);
                    }
                }
            });
            invoice.add_suffix(&pay);
        }
        let (c, id, m) = (ctx.clone(), s.card.id, month.to_string());
        invoice.connect_activated(move |_| c.nav.push(&card_page(&c, id, &m)));
        list.append(&invoice);

        if s.card.credit_limit > 0.0 {
            let usage = adw::ActionRow::builder()
                .title("Limite")
                .subtitle(format!(
                    "{} usado de {} · disponível {}",
                    money(ctx, s.open_total),
                    money(ctx, s.card.credit_limit),
                    money(ctx, s.available())
                ))
                .build();
            let bar = gtk::ProgressBar::new();
            bar.set_fraction(s.usage().clamp(0.0, 1.0));
            bar.set_valign(gtk::Align::Center);
            bar.set_size_request(120, -1);
            bar.add_css_class("thick");
            bar.add_css_class(usage_class(s.usage()));
            usage.add_suffix(&bar);
            list.append(&usage);
        }

        let buy = button_row("Nova compra", Some("list-add-symbolic"));
        let (c, card, m) = (ctx.clone(), s.card.clone(), month.to_string());
        buy.connect_activated(move |_| purchase_dialog(&c, &card, &m));
        list.append(&buy);
        group.add(&list);
        b.append(&group);
    }
}

fn usage_class(fraction: f64) -> &'static str {
    if fraction > 1.0 {
        "bar-color-1001"
    } else if fraction >= 0.8 {
        "bar-color-1009"
    } else {
        "bar-color-1002"
    }
}

/// Fatura de um cartão (com navegação entre meses) e as compras dela.
fn card_page(ctx: &Shared, card_id: i64, month: &str) -> adw::NavigationPage {
    let month = Rc::new(RefCell::new(month.to_string()));
    let content = widgets::content_box();
    let title = adw::WindowTitle::new("", "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));
    let nav = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    nav.add_css_class("linked");
    let prev = widgets::icon_button("go-previous-symbolic", "Fatura anterior");
    let next = widgets::icon_button("go-next-symbolic", "Próxima fatura");
    nav.append(&prev);
    nav.append(&next);
    header.pack_end(&nav);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&widgets::scroller(&content)));
    let page = adw::NavigationPage::builder().title("Fatura").child(&toolbar).build();

    let fill: Rc<dyn Fn() -> bool> = {
        let (c, wc, wt, m) = (ctx.clone(), content.downgrade(), title.downgrade(), month.clone());
        Rc::new(move || {
            let (Some(content), Some(title)) = (wc.upgrade(), wt.upgrade()) else { return false };
            widgets::clear_box(&content);
            let Some(card) = c.check(c.db.fin_card(card_id)).flatten() else {
                c.nav.pop();
                return false;
            };
            let month = m.borrow().clone();
            title.set_title(&card.name);
            title.set_subtitle(&format!("Fatura de {}", fin::month_label(&month)));
            fill_card_page(&c, &content, &card, &month);
            true
        })
    };
    for (button, delta) in [(&prev, -1), (&next, 1)] {
        let (m, fl) = (month.clone(), fill.clone());
        button.connect_clicked(move |_| {
            let next = fin::add_months(&m.borrow(), delta);
            *m.borrow_mut() = next;
            let fl = fl.clone();
            glib::idle_add_local_once(move || {
                fl();
            });
        });
    }
    fill();
    let fl = fill.clone();
    ctx.on_refresh(move || fl());
    page
}

fn fill_card_page(ctx: &Shared, content: &gtk::Box, card: &Card, month: &str) {
    let purchases = ctx.check(ctx.db.fin_purchases(card.id, month)).unwrap_or_default();
    let invoice = ctx.check(ctx.db.fin_invoice_entry(card.id, month)).flatten();
    let look = Lookup::load(ctx);
    let total: f64 = purchases.iter().map(|p| p.amount).sum();

    let stats = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(3)
        .column_spacing(12)
        .row_spacing(12)
        .build();
    stats.append(&crate::home::stat_card(&money(ctx, total), "total da fatura", "unloop-card-symbolic"));
    stats.append(&crate::home::stat_card(
        &fin::date_short(&fin::day_in_month(month, card.due_day)),
        if invoice.as_ref().is_some_and(|e| e.paid) { "vencimento · paga ✓" } else { "vencimento" },
        "x-office-calendar-symbolic",
    ));
    stats.append(&crate::home::stat_card(&purchases.len().to_string(), "compras", "view-list-bullet-symbolic"));
    content.append(&stats);

    if let Some(e) = &invoice {
        let pay = widgets::pill_button(
            if e.paid { "Marcar como não paga" } else { "Pagar fatura" },
            if e.paid { None } else { Some("suggested-action") },
        );
        pay.set_halign(gtk::Align::Center);
        let (c, id, paid) = (ctx.clone(), e.id, e.paid);
        pay.connect_clicked(move |_| {
            if c.check(c.db.fin_set_paid(id, !paid, None)).is_some() {
                changed_later(&c);
            }
        });
        content.append(&pay);
    }

    let group = widgets::group("Compras", None);
    let add = suffix_button("Nova compra", "list-add-symbolic");
    let (c, cd, m) = (ctx.clone(), card.clone(), month.to_string());
    add.connect_clicked(move |_| purchase_dialog(&c, &cd, &m));
    group.set_header_suffix(Some(&add));
    if purchases.is_empty() {
        group.set_description(Some("Nenhuma compra nesta fatura."));
    } else {
        let list = widgets::boxed_list();
        for p in &purchases {
            let mut parts = vec![fin::date_short(&p.date)];
            if let Some(cat) = look.cat(p.category_id) {
                parts.push(cat.name.clone());
            }
            let row = adw::ActionRow::builder().title(esc(&p.description)).subtitle(esc(&parts.join(" · "))).build();
            if let Some(cat) = look.cat(p.category_id) {
                row.add_prefix(&dot(cat.color));
            }
            row.add_suffix(&amount_label(&money(ctx, p.amount), None));
            let delete = widgets::icon_button("user-trash-symbolic", "Apagar compra");
            let (c, purchase) = (ctx.clone(), p.clone());
            delete.connect_clicked(move |_| {
                let id = purchase.id;
                if purchase.installment_total.is_some_and(|n| n > 1) {
                    let d = adw::MessageDialog::new(
                        Some(&c.window),
                        Some("Apagar compra parcelada?"),
                        Some("Apague só esta parcela ou a compra inteira (todas as faturas)."),
                    );
                    d.add_responses(&[("cancel", "Cancelar"), ("one", "Só esta parcela"), ("all", "Todas as parcelas")]);
                    d.set_response_appearance("all", adw::ResponseAppearance::Destructive);
                    d.set_close_response("cancel");
                    let c2 = c.clone();
                    d.connect_response(None, move |_, r| {
                        if (r == "one" || r == "all") && c2.check(c2.db.fin_delete_purchase(id, r == "all")).is_some() {
                            c2.changed();
                        }
                    });
                    d.present();
                } else if c.check(c.db.fin_delete_purchase(id, false)).is_some() {
                    changed_later(&c);
                }
            });
            row.add_suffix(&delete);
            list.append(&row);
        }
        group.add(&list);
    }
    content.append(&group);
}

fn category_combo(cats: &[Category], kind: Kind, current: Option<i64>) -> (adw::ComboRow, Vec<Category>) {
    let options: Vec<Category> = cats.iter().filter(|c| c.kind == kind).cloned().collect();
    let mut labels = vec!["Sem categoria".to_string()];
    labels.extend(options.iter().map(|c| c.name.clone()));
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let row = adw::ComboRow::builder().title("Categoria").model(&gtk::StringList::new(&refs)).build();
    row.set_selected(current.and_then(|id| options.iter().position(|c| c.id == id)).map_or(0, |i| i + 1) as u32);
    (row, options)
}

fn purchase_dialog(ctx: &Shared, card: &Card, month: &str) {
    let cats = ctx.check(ctx.db.fin_categories()).unwrap_or_default();
    let form = widgets::boxed_list();
    let description = adw::EntryRow::builder().title("Descrição").build();
    let amount = money_row("Valor total", 0.0, false);
    let initial = if month == fin::current_month() { today_key() } else { fin::day_in_month(month, 1) };
    let (date_row, date) = date_row("Data da compra", Some(initial), false);
    let (category, options) = category_combo(&cats, Kind::Expense, None);
    let installments = adw::SpinRow::with_range(1.0, 48.0, 1.0);
    installments.set_title("Parcelas");
    installments.set_value(1.0);
    form.append(&description);
    form.append(&amount);
    form.append(&date_row);
    form.append(&category);
    form.append(&installments);
    let (c, card) = (ctx.clone(), card.clone());
    form_dialog(
        ctx,
        &format!("Compra no {}", card.name),
        "Parcelada, cada parcela cai numa fatura seguinte.",
        &form,
        &[("ok", "Lançar", adw::ResponseAppearance::Suggested)],
        move |_| {
            let desc = description.text().trim().to_string();
            if desc.is_empty() || amount.value() <= 0.0 {
                c.toast("Informe a descrição e um valor maior que zero.");
                return;
            }
            let d = date.borrow().clone().unwrap_or_else(today_key);
            let cat = (category.selected() as usize).checked_sub(1).and_then(|i| options.get(i)).map(|c| c.id);
            let n = installments.value().round().max(1.0) as u32;
            if c.check(c.db.fin_add_purchase(card.id, &desc, amount.value(), &d, cat, n)).is_some() {
                let invoice = fin::parse_date(&d).map(|day| card.invoice_for(day)).unwrap_or_default();
                c.toast(&format!("Compra lançada na fatura de {}.", fin::month_short(&invoice)));
                c.changed();
            }
        },
    );
}

fn card_editor(ctx: &Shared, card: Option<Card>) -> adw::NavigationPage {
    let editing = card.is_some();
    let original = card.unwrap_or_else(|| Card::new(""));
    let accounts = ctx.check(ctx.db.fin_accounts()).unwrap_or_default();
    let prefs = adw::PreferencesPage::new();
    let main = widgets::group("Cartão", None);
    let name = adw::EntryRow::builder().title("Nome").build();
    name.set_text(&original.name);
    let limit = money_row("Limite", original.credit_limit, false);
    let closing = adw::SpinRow::with_range(1.0, 31.0, 1.0);
    closing.set_title("Dia do fechamento");
    closing.set_subtitle("Compras depois deste dia vão para a fatura seguinte");
    closing.set_value(f64::from(original.closing_day));
    let due = adw::SpinRow::with_range(1.0, 31.0, 1.0);
    due.set_title("Dia do vencimento");
    due.set_value(f64::from(original.due_day));
    let mut labels = vec!["Nenhuma".to_string()];
    labels.extend(accounts.iter().map(|a| a.name.clone()));
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let account = adw::ComboRow::builder()
        .title("Conta que paga a fatura")
        .model(&gtk::StringList::new(&refs))
        .build();
    account.set_selected(
        original.account_id.and_then(|id| accounts.iter().position(|a| a.id == id)).map_or(0, |i| i + 1) as u32,
    );
    let color_row = color_combo("Cor", original.color);
    main.add(&name);
    main.add(&limit);
    main.add(&closing);
    main.add(&due);
    main.add(&account);
    main.add(&color_row);
    prefs.add(&main);

    if editing {
        let actions = widgets::group("", None);
        let delete = widgets::pill_button("Apagar cartão", Some("destructive-action"));
        delete.set_halign(gtk::Align::Center);
        let (c, id) = (ctx.clone(), original.id);
        delete.connect_clicked(move |_| {
            let c2 = c.clone();
            widgets::confirm(
                &c,
                "Apagar cartão?",
                "As compras e as faturas em aberto serão apagadas. Faturas já pagas continuam como despesas.",
                "Apagar",
                true,
                move || {
                    if c2.check(c2.db.fin_delete_card(id)).is_some() {
                        c2.nav.pop();
                        c2.changed();
                    }
                },
            );
        });
        actions.add(&delete);
        prefs.add(&actions);
    }

    let header = adw::HeaderBar::new();
    let save = gtk::Button::with_label("Salvar");
    save.add_css_class("suggested-action");
    header.pack_end(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&prefs));
    let page = adw::NavigationPage::builder()
        .title(if editing { "Editar cartão" } else { "Novo cartão" })
        .child(&toolbar)
        .build();
    let c = ctx.clone();
    save.connect_clicked(move |_| {
        let n = name.text().trim().to_string();
        if n.is_empty() {
            c.toast("Dê um nome ao cartão.");
            return;
        }
        let card = Card {
            name: n,
            credit_limit: limit.value(),
            closing_day: closing.value() as u32,
            due_day: due.value() as u32,
            account_id: (account.selected() as usize).checked_sub(1).and_then(|i| accounts.get(i)).map(|a| a.id),
            color: color_from(&color_row),
            ..original.clone()
        };
        let ok = if editing {
            c.check(c.db.fin_update_card(&card)).is_some()
        } else {
            c.check(c.db.fin_insert_card(&card)).is_some()
        };
        if ok {
            c.changed();
            c.nav.pop();
        }
    });
    page
}

// --------------------------------------------------------------- Orçamento

fn fill_budget(f: &Rc<Fin>, b: &gtk::Box, month: &str) {
    let ctx = &f.ctx;
    let lines = ctx.check(ctx.db.fin_budget(month)).unwrap_or_default();
    let add = suffix_button("Definir limite", "list-add-symbolic");
    let c = ctx.clone();
    add.connect_clicked(move |_| budget_dialog(&c, None));
    let limit: f64 = lines.iter().map(|l| l.limit).sum();
    let spent: f64 = lines.iter().map(|l| l.spent).sum();
    let description = if lines.is_empty() {
        "Defina um limite mensal por categoria de despesa e acompanhe quanto já foi gasto.".to_string()
    } else {
        format!("{} gasto de {} planejado", money(ctx, spent), money(ctx, limit))
    };
    let group = widgets::group("Orçamento por categoria", Some(description.as_str()));
    group.set_header_suffix(Some(&add));
    if !lines.is_empty() {
        let list = widgets::boxed_list();
        for l in &lines {
            list.append(&budget_row(ctx, l));
        }
        group.add(&list);
    }
    b.append(&group);
    if lines.is_empty() {
        empty(b, "unloop-pie-symbolic", "Sem orçamento", "Compras no cartão contam na categoria delas.", None);
        return;
    }

    // Categorias com gasto mas sem limite.
    let spending = ctx.check(ctx.db.fin_spending_by_category(month)).unwrap_or_default();
    let cats = ctx.check(ctx.db.fin_categories()).unwrap_or_default();
    let mut loose: Vec<(String, f64)> = spending
        .iter()
        .filter(|(id, v)| **v > 0.0 && !id.is_some_and(|id| lines.iter().any(|l| l.category.id == id)))
        .map(|(id, v)| {
            (id.and_then(|id| cats.iter().find(|c| c.id == id)).map_or("Sem categoria".into(), |c| c.name.clone()), *v)
        })
        .collect();
    loose.sort_by(|a, b| b.1.total_cmp(&a.1));
    if !loose.is_empty() {
        let group = widgets::group("Gastos sem limite definido", None);
        let list = widgets::boxed_list();
        for (name, v) in loose {
            let (row, _) = widgets::value_row(&name, &money(ctx, v));
            list.append(&row);
        }
        group.add(&list);
        b.append(&group);
    }
}

fn budget_row(ctx: &Shared, l: &BudgetLine) -> adw::ActionRow {
    let rest = l.limit - l.spent;
    let subtitle = if rest >= 0.0 {
        format!("{} de {} · restam {}", money(ctx, l.spent), money(ctx, l.limit), money(ctx, rest))
    } else {
        format!("{} de {} · passou {}", money(ctx, l.spent), money(ctx, l.limit), money(ctx, -rest))
    };
    let row = adw::ActionRow::builder()
        .title(esc(&l.category.name))
        .subtitle(esc(&subtitle))
        .activatable(true)
        .build();
    row.add_prefix(&dot(l.category.color));
    let pct = gtk::Label::new(Some(&format!("{:.0}%", l.fraction() * 100.0)));
    pct.add_css_class("numeric");
    if l.fraction() > 1.0 {
        pct.add_css_class("error");
    }
    let bar = gtk::ProgressBar::new();
    bar.set_fraction(l.fraction().clamp(0.0, 1.0));
    bar.set_valign(gtk::Align::Center);
    bar.set_size_request(140, -1);
    bar.add_css_class("thick");
    bar.add_css_class(usage_class(l.fraction()));
    row.add_suffix(&bar);
    row.add_suffix(&pct);
    let (c, line) = (ctx.clone(), l.clone());
    row.connect_activated(move |_| budget_dialog(&c, Some(line.clone())));
    row
}

fn budget_dialog(ctx: &Shared, line: Option<BudgetLine>) {
    let cats: Vec<Category> =
        ctx.check(ctx.db.fin_categories()).unwrap_or_default().into_iter().filter(|c| c.kind == Kind::Expense).collect();
    if cats.is_empty() {
        widgets::info(ctx, "Orçamento", "Crie uma categoria de despesa primeiro.");
        return;
    }
    let form = widgets::boxed_list();
    let names: Vec<&str> = cats.iter().map(|c| c.name.as_str()).collect();
    let category = adw::ComboRow::builder().title("Categoria").model(&gtk::StringList::new(&names)).build();
    if let Some(l) = &line {
        category.set_selected(cats.iter().position(|c| c.id == l.category.id).unwrap_or(0) as u32);
        category.set_sensitive(false);
    }
    let limit = money_row("Limite por mês", line.as_ref().map_or(0.0, |l| l.limit), false);
    form.append(&category);
    form.append(&limit);
    let mut actions = vec![("save", "Salvar", adw::ResponseAppearance::Suggested)];
    if line.is_some() {
        actions.insert(0, ("remove", "Remover limite", adw::ResponseAppearance::Destructive));
    }
    let c = ctx.clone();
    form_dialog(ctx, "Limite da categoria", "", &form, &actions, move |response| {
        let Some(cat) = cats.get(category.selected() as usize) else { return };
        let value = if response == "remove" { 0.0 } else { limit.value() };
        if c.check(c.db.fin_set_budget(cat.id, value)).is_some() {
            c.changed();
        }
    });
}

// ------------------------------------------------------------------- Metas

fn pace_text(p: Pace) -> &'static str {
    match p {
        Pace::Done => "🎉 Concluída",
        Pace::Paused => "⏸ Pausada",
        Pace::Archived => "Arquivada",
        Pace::Ahead => "✅ No ritmo",
        Pace::Behind => "⚠️ Atrasada",
        Pace::Free => "",
    }
}

fn goal_subtitle(ctx: &Shared, g: &Goal) -> String {
    let st = fin::goal_status(g, &fin::current_month());
    let mut parts = vec![format!("{} de {}", money(ctx, g.saved), money(ctx, g.target))];
    if st.missing > 0.0 && !matches!(st.pace, Pace::Done | Pace::Archived) {
        match (&g.deadline, st.months_left) {
            (Some(d), Some(m)) if m > 0 => {
                parts.push(format!("guarde {}/mês até {}", money(ctx, st.per_month), fin::month_short(d)))
            }
            (Some(_), _) => parts.push(format!("faltam {} (prazo é agora)", money(ctx, st.missing))),
            _ => parts.push(format!("faltam {}", money(ctx, st.missing))),
        }
    }
    let pace = pace_text(st.pace);
    if !pace.is_empty() {
        parts.push(pace.to_string());
    }
    parts.join(" · ")
}

fn goal_row(ctx: &Shared, g: &Goal) -> adw::ActionRow {
    let st = fin::goal_status(g, &fin::current_month());
    let row = adw::ActionRow::builder()
        .title(esc(&g.name))
        .subtitle(esc(&goal_subtitle(ctx, g)))
        .activatable(true)
        .build();
    row.add_prefix(&emoji_avatar(&g.icon, g.color));
    let bar = gtk::ProgressBar::new();
    bar.set_fraction(st.progress);
    bar.set_valign(gtk::Align::Center);
    bar.set_size_request(120, -1);
    bar.add_css_class("thick");
    bar.add_css_class(&format!("bar-color-{}", color(g.color).code));
    row.add_suffix(&bar);
    let pct = gtk::Label::new(Some(&format!("{:.0}%", st.progress * 100.0)));
    pct.add_css_class("numeric");
    row.add_suffix(&pct);
    if g.state != GoalState::Active {
        row.add_css_class("dim-label");
    }
    let (c, id) = (ctx.clone(), g.id);
    row.connect_activated(move |_| c.nav.push(&goal_page(&c, id)));
    row
}

fn fill_goals(f: &Rc<Fin>, b: &gtk::Box, goals: &[Goal]) {
    let ctx = &f.ctx;
    let add = suffix_button("Nova meta", "list-add-symbolic");
    let c = ctx.clone();
    add.connect_clicked(move |_| c.nav.push(&goal_editor(&c, None)));
    let active: Vec<&Goal> = goals.iter().filter(|g| matches!(g.state, GoalState::Active | GoalState::Paused)).collect();
    let saved: f64 = active.iter().map(|g| g.saved).sum();
    let group = widgets::group(
        "Metas de economia",
        Some(&if active.is_empty() {
            "Junte dinheiro para algo e veja se está no ritmo.".to_string()
        } else {
            format!("{} guardado nas metas em andamento", money(ctx, saved))
        }),
    );
    group.set_header_suffix(Some(&add));
    if !active.is_empty() {
        let list = widgets::boxed_list();
        for g in &active {
            list.append(&goal_row(ctx, g));
        }
        group.add(&list);
    }
    b.append(&group);

    let done: Vec<&Goal> = goals.iter().filter(|g| g.state == GoalState::Done).collect();
    if !done.is_empty() {
        let group = widgets::group("Concluídas", None);
        let list = widgets::boxed_list();
        for g in done {
            list.append(&goal_row(ctx, g));
        }
        group.add(&list);
        b.append(&group);
    }

    let archived: Vec<&Goal> = goals.iter().filter(|g| g.state == GoalState::Archived).collect();
    if !archived.is_empty() {
        let toggle = gtk::Button::with_label(&if f.st.show_archived.get() {
            "Esconder arquivadas".to_string()
        } else {
            format!("Mostrar arquivadas ({})", archived.len())
        });
        toggle.add_css_class("flat");
        toggle.set_halign(gtk::Align::Center);
        let fc = f.clone();
        toggle.connect_clicked(move |_| {
            fc.st.show_archived.set(!fc.st.show_archived.get());
            rerender(&fc);
        });
        b.append(&toggle);
        if f.st.show_archived.get() {
            let group = widgets::group("Arquivadas", None);
            let list = widgets::boxed_list();
            for g in archived {
                list.append(&goal_row(ctx, g));
            }
            group.add(&list);
            b.append(&group);
        }
    }

    if goals.is_empty() {
        let create = widgets::pill_button("Criar minha primeira meta", Some("suggested-action"));
        create.set_halign(gtk::Align::Center);
        let c = ctx.clone();
        create.connect_clicked(move |_| c.nav.push(&goal_editor(&c, None)));
        empty(
            b,
            "unloop-target-symbolic",
            "Nenhuma meta ainda",
            "Reserva de emergência, viagem, um notebook novo… Defina o valor e, se quiser, um prazo.",
            Some(create.upcast_ref()),
        );
    }
}

fn goal_page(ctx: &Shared, goal_id: i64) -> adw::NavigationPage {
    let content = widgets::content_box();
    let header = adw::HeaderBar::new();
    let edit = widgets::icon_button("document-edit-symbolic", "Editar meta");
    header.pack_end(&edit);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&widgets::scroller(&content)));
    let page = adw::NavigationPage::builder().title("Meta").child(&toolbar).build();
    let c = ctx.clone();
    edit.connect_clicked(move |_| {
        if let Some(g) = c.check(c.db.fin_goal(goal_id)).flatten() {
            c.nav.push(&goal_editor(&c, Some(g)));
        }
    });

    let fill = {
        let (c, wc, wp) = (ctx.clone(), content.downgrade(), page.downgrade());
        move || -> bool {
            let (Some(content), Some(page)) = (wc.upgrade(), wp.upgrade()) else { return false };
            widgets::clear_box(&content);
            let Some(g) = c.check(c.db.fin_goal(goal_id)).flatten() else { return false };
            page.set_title(&g.name);
            fill_goal_page(&c, &content, &g);
            true
        }
    };
    fill();
    ctx.on_refresh(fill);
    page
}

fn fill_goal_page(ctx: &Shared, content: &gtk::Box, g: &Goal) {
    let st = fin::goal_status(g, &fin::current_month());
    let top = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let avatar = emoji_avatar(&g.icon, g.color);
    avatar.add_css_class("large");
    avatar.set_halign(gtk::Align::Center);
    top.append(&avatar);
    top.append(&widgets::label(&g.name, &["title-2"]));
    top.append(&widgets::label(&money(ctx, g.saved), &["counter"]));
    top.append(&widgets::label(&format!("de {} · {:.0}%", money(ctx, g.target), st.progress * 100.0), &["dim-label"]));
    let bar = gtk::ProgressBar::new();
    bar.set_fraction(st.progress);
    bar.add_css_class("thick");
    bar.add_css_class(&format!("bar-color-{}", color(g.color).code));
    bar.set_margin_start(24);
    bar.set_margin_end(24);
    top.append(&bar);
    content.append(&top);

    let info = widgets::group("Situação", None);
    let list = widgets::boxed_list();
    let (r, _) = widgets::value_row("Estado", g.state.label());
    list.append(&r);
    let (r, _) = widgets::value_row("Falta", &money(ctx, st.missing));
    list.append(&r);
    if let Some(d) = &g.deadline {
        let (r, _) = widgets::value_row("Prazo", &fin::month_label(d));
        list.append(&r);
        if st.missing > 0.0 {
            let (r, _) = widgets::value_row("Guardar por mês", &money(ctx, st.per_month));
            list.append(&r);
        }
        if let Some(e) = st.expected {
            let (r, _) = widgets::value_row("Esperado até agora", &money(ctx, e));
            list.append(&r);
        }
    }
    let pace = pace_text(st.pace);
    if !pace.is_empty() {
        let (r, _) = widgets::value_row("Ritmo", pace);
        list.append(&r);
    }
    info.add(&list);
    content.append(&info);

    // Ações de estado.
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    buttons.set_halign(gtk::Align::Center);
    let deposit = widgets::pill_button("Depositar", Some("suggested-action"));
    let (c, id) = (ctx.clone(), g.id);
    deposit.connect_clicked(move |_| deposit_dialog(&c, id));
    buttons.append(&deposit);
    let state_actions: &[(GoalState, &str)] = match g.state {
        GoalState::Active => &[(GoalState::Paused, "Pausar"), (GoalState::Done, "Concluir")],
        GoalState::Paused => &[(GoalState::Active, "Retomar"), (GoalState::Archived, "Arquivar")],
        GoalState::Done => &[(GoalState::Active, "Reabrir"), (GoalState::Archived, "Arquivar")],
        GoalState::Archived => &[(GoalState::Active, "Desarquivar")],
    };
    for (state, label) in state_actions {
        let b = widgets::pill_button(label, None);
        let (c, id, s) = (ctx.clone(), g.id, *state);
        b.connect_clicked(move |_| {
            if c.check(c.db.fin_set_goal_state(id, s)).is_some() {
                changed_later(&c);
            }
        });
        buttons.append(&b);
    }
    content.append(&buttons);

    let deposits = ctx.check(ctx.db.fin_deposits(g.id)).unwrap_or_default();
    let group = widgets::group("Depósitos", Some(if deposits.is_empty() { "Nenhum depósito ainda." } else { "" }).filter(|s| !s.is_empty()));
    if !deposits.is_empty() {
        let list = widgets::boxed_list();
        for d in &deposits {
            let row = adw::ActionRow::builder()
                .title(fin::date_short(&d.date))
                .subtitle(esc(&d.note))
                .build();
            row.add_suffix(&amount_label(
                &if d.amount < 0.0 { format!("−{}", money(ctx, -d.amount)) } else { format!("+{}", money(ctx, d.amount)) },
                Some(if d.amount < 0.0 { "error" } else { "success" }),
            ));
            let delete = widgets::icon_button("user-trash-symbolic", "Apagar depósito");
            let (c, id) = (ctx.clone(), d.id);
            delete.connect_clicked(move |_| {
                if c.check(c.db.fin_delete_deposit(id)).is_some() {
                    changed_later(&c);
                }
            });
            row.add_suffix(&delete);
            list.append(&row);
        }
        group.add(&list);
    }
    content.append(&group);
}

fn deposit_dialog(ctx: &Shared, goal_id: i64) {
    let form = widgets::boxed_list();
    let amount = money_row("Valor", 0.0, false);
    let withdraw = adw::SwitchRow::builder().title("É uma retirada").build();
    let (date_row, date) = date_row("Data", Some(today_key()), false);
    let note = adw::EntryRow::builder().title("Observação (opcional)").build();
    form.append(&amount);
    form.append(&withdraw);
    form.append(&date_row);
    form.append(&note);
    let c = ctx.clone();
    form_dialog(ctx, "Depósito na meta", "", &form, &[("ok", "Salvar", adw::ResponseAppearance::Suggested)], move |_| {
        if amount.value() <= 0.0 {
            c.toast("Informe um valor maior que zero.");
            return;
        }
        let value = if withdraw.is_active() { -amount.value() } else { amount.value() };
        let d = date.borrow().clone().unwrap_or_else(today_key);
        if let Some(total) = c.check(c.db.fin_add_deposit(goal_id, value, &d, &note.text())) {
            c.toast(&format!("Guardado na meta: {}", money(&c, total)));
            c.changed();
        }
    });
}

fn goal_editor(ctx: &Shared, goal: Option<Goal>) -> adw::NavigationPage {
    let editing = goal.is_some();
    let original = goal.unwrap_or_else(|| Goal::new("", 0.0));
    let prefs = adw::PreferencesPage::new();
    let main = widgets::group("Meta", None);
    let name = adw::EntryRow::builder().title("Nome (ex.: Reserva de emergência)").build();
    name.set_text(&original.name);
    let target = money_row("Valor alvo", original.target, false);
    let icon = adw::EntryRow::builder().title("Emoji").build();
    icon.set_text(&original.icon);
    let color_row = color_combo("Cor", original.color);
    main.add(&name);
    main.add(&target);
    main.add(&icon);
    main.add(&color_row);
    prefs.add(&main);

    let when = widgets::group("Prazo", Some("Com prazo, o app diz quanto guardar por mês e se você está no ritmo."));
    let has_deadline = adw::SwitchRow::builder().title("Tem prazo").active(original.deadline.is_some()).build();
    let (dy, dm) = original
        .deadline
        .as_deref()
        .and_then(fin::parse_month)
        .unwrap_or_else(|| fin::parse_month(&fin::add_months(&fin::current_month(), 12)).unwrap_or((2027, 1)));
    let month_row = adw::ComboRow::builder().title("Mês").model(&gtk::StringList::new(&fin::MONTH_NAMES)).build();
    month_row.set_selected(dm - 1);
    let year_row = adw::SpinRow::with_range(2000.0, 2100.0, 1.0);
    year_row.set_title("Ano");
    year_row.set_value(f64::from(dy));
    has_deadline.bind_property("active", &month_row, "sensitive").sync_create().build();
    has_deadline.bind_property("active", &year_row, "sensitive").sync_create().build();
    when.add(&has_deadline);
    when.add(&month_row);
    when.add(&year_row);
    prefs.add(&when);

    if editing {
        let actions = widgets::group("", None);
        let delete = widgets::pill_button("Apagar meta", Some("destructive-action"));
        delete.set_halign(gtk::Align::Center);
        let (c, id) = (ctx.clone(), original.id);
        delete.connect_clicked(move |_| {
            let c2 = c.clone();
            widgets::confirm(&c, "Apagar meta?", "A meta e todos os depósitos serão apagados.", "Apagar", true, move || {
                if c2.check(c2.db.fin_delete_goal(id)).is_some() {
                    c2.nav.pop();
                    c2.nav.pop();
                    c2.changed();
                }
            });
        });
        actions.add(&delete);
        prefs.add(&actions);
    }

    let header = adw::HeaderBar::new();
    let save = gtk::Button::with_label("Salvar");
    save.add_css_class("suggested-action");
    header.pack_end(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&prefs));
    let page = adw::NavigationPage::builder()
        .title(if editing { "Editar meta" } else { "Nova meta" })
        .child(&toolbar)
        .build();
    let c = ctx.clone();
    save.connect_clicked(move |_| {
        let n = name.text().trim().to_string();
        if n.is_empty() || target.value() <= 0.0 {
            c.toast("Dê um nome e um valor alvo maior que zero.");
            return;
        }
        let emoji: String = icon.text().trim().chars().take(8).collect();
        let g = Goal {
            name: n,
            target: target.value(),
            icon: if emoji.is_empty() { "🎯".into() } else { emoji },
            color: color_from(&color_row),
            deadline: has_deadline
                .is_active()
                .then(|| format!("{:04}-{:02}", year_row.value() as i32, month_row.selected() + 1)),
            ..original.clone()
        };
        let ok = if editing {
            c.check(c.db.fin_update_goal(&g)).is_some()
        } else {
            c.check(c.db.fin_insert_goal(&g)).is_some()
        };
        if ok {
            c.changed();
            c.nav.pop();
        }
    });
    page
}

// -------------------------------------------------------------- Categorias

fn fill_categories(f: &Rc<Fin>, b: &gtk::Box) {
    let ctx = &f.ctx;
    let cats = ctx.check(ctx.db.fin_categories()).unwrap_or_default();
    for kind in [Kind::Expense, Kind::Income] {
        let title = if kind == Kind::Expense { "Despesas" } else { "Receitas" };
        let group = widgets::group(title, None);
        let list = widgets::boxed_list();
        for cat in cats.iter().filter(|c| c.kind == kind) {
            let row = adw::ActionRow::builder().title(esc(&cat.name)).activatable(true).build();
            row.add_prefix(&dot(cat.color));
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            let (c, cat) = (ctx.clone(), cat.clone());
            row.connect_activated(move |_| category_dialog(&c, Some(cat.clone()), kind));
            list.append(&row);
        }
        let add = adw::EntryRow::builder().title("Nova categoria (Enter)").show_apply_button(true).build();
        let c = ctx.clone();
        add.connect_apply(move |row| {
            let name = row.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let palette = [1001, 1010, 1009, 1002, 1008, 1003, 1006, 1005, 1004, 1011];
            let color = palette[(name.len() + name.chars().next().map_or(0, |ch| ch as usize)) % palette.len()];
            if c.check(c.db.fin_insert_category(&name, kind, color)).is_some() {
                row.set_text("");
                changed_later(&c);
            }
        });
        list.append(&add);
        group.add(&list);
        b.append(&group);
    }
}

fn category_dialog(ctx: &Shared, cat: Option<Category>, kind: Kind) {
    let Some(cat) = cat else { return };
    let form = widgets::boxed_list();
    let name = adw::EntryRow::builder().title("Nome").build();
    name.set_text(&cat.name);
    let color_row = color_combo("Cor", cat.color);
    form.append(&name);
    form.append(&color_row);
    let c = ctx.clone();
    form_dialog(
        ctx,
        &format!("Categoria de {}", kind.label().to_lowercase()),
        "",
        &form,
        &[("delete", "Apagar", adw::ResponseAppearance::Destructive), ("save", "Salvar", adw::ResponseAppearance::Suggested)],
        move |response| {
            if response == "delete" {
                let c2 = c.clone();
                let id = cat.id;
                widgets::confirm(
                    &c,
                    "Apagar categoria?",
                    "Os lançamentos dela ficam sem categoria e o limite do orçamento some.",
                    "Apagar",
                    true,
                    move || {
                        if c2.check(c2.db.fin_delete_category(id)).is_some() {
                            c2.changed();
                        }
                    },
                );
                return;
            }
            let n = name.text().trim().to_string();
            if n.is_empty() {
                return;
            }
            let updated = Category { name: n, color: color_from(&color_row), ..cat.clone() };
            if c.check(c.db.fin_update_category(&updated)).is_some() {
                c.changed();
            }
        },
    );
}

// ------------------------------------------------------------------ Datas

/// Linha com data escolhida num calendário (popover). `optional` permite limpar.
fn date_row(title: &str, initial: Option<String>, optional: bool) -> (adw::ActionRow, Rc<RefCell<Option<String>>>) {
    let value = Rc::new(RefCell::new(initial));
    let row = adw::ActionRow::builder().title(title).build();
    let describe = |v: &Option<String>| {
        v.as_deref().and_then(fin::parse_date).map_or_else(|| "Sem data".to_string(), |d| d.format("%d/%m/%Y").to_string())
    };
    let calendar = gtk::Calendar::new();
    if let Some(d) = value.borrow().as_deref().and_then(fin::parse_date) {
        if let Ok(dt) = glib::DateTime::from_local(d.year(), d.month() as i32, d.day() as i32, 12, 0, 0.0) {
            calendar.select_day(&dt);
        }
    }
    let popover = gtk::Popover::new();
    popover.set_child(Some(&calendar));
    let button = gtk::MenuButton::builder()
        .label(describe(&value.borrow()))
        .popover(&popover)
        .valign(gtk::Align::Center)
        .build();
    row.add_suffix(&button);
    row.set_activatable_widget(Some(&button));
    let (v, wb, wp) = (value.clone(), button.downgrade(), popover.downgrade());
    calendar.connect_day_selected(move |cal| {
        let d = cal.date();
        let key = format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day_of_month());
        if let Some(b) = wb.upgrade() {
            b.set_label(&describe(&Some(key.clone())));
        }
        *v.borrow_mut() = Some(key);
        if let Some(p) = wp.upgrade() {
            p.popdown();
        }
    });
    if optional {
        let clear = widgets::icon_button("edit-clear-symbolic", "Sem data");
        let (v, wb) = (value.clone(), button.downgrade());
        clear.connect_clicked(move |_| {
            *v.borrow_mut() = None;
            if let Some(b) = wb.upgrade() {
                b.set_label("Sem data");
            }
        });
        row.add_suffix(&clear);
    }
    (row, value)
}

// ---------------------------------------------------------- Editor de lançamento

const REPEATS: [&str; 4] = ["Não repete", "Todo mês", "Todo mês, por um número de meses", "Parcelado"];

/// Editor de lançamento. `e.id == 0` = novo (já pode vir com mês, data e tipo).
pub fn entry_editor(ctx: &Shared, e: Entry) -> adw::NavigationPage {
    let editing = e.id != 0;
    let original = Rc::new(e);
    let look = Lookup::load(ctx);
    let prefs = adw::PreferencesPage::new();

    if original.is_transfer() {
        return transfer_page(ctx, &original, &look);
    }

    let main = widgets::group("Lançamento", None);
    let kind_row = adw::ComboRow::builder()
        .title("Tipo")
        .model(&gtk::StringList::new(&["Despesa", "Receita"]))
        .build();
    kind_row.set_selected(if original.kind == Kind::Income { 1 } else { 0 });
    let description = adw::EntryRow::builder().title("Descrição").build();
    description.set_text(&original.description);
    let amount = money_row("Valor", original.amount, false);
    main.add(&kind_row);
    main.add(&description);
    main.add(&amount);

    let (category_row, options) = category_combo(&look.cats, original.kind, original.category_id);
    let options = Rc::new(RefCell::new(options));
    main.add(&category_row);
    let mut account_labels = vec!["Sem conta".to_string()];
    account_labels.extend(look.accounts.iter().map(|a| a.name.clone()));
    let refs: Vec<&str> = account_labels.iter().map(String::as_str).collect();
    let account_row = adw::ComboRow::builder().title("Conta").model(&gtk::StringList::new(&refs)).build();
    account_row.set_selected(
        original.account_id.and_then(|id| look.accounts.iter().position(|a| a.id == id)).map_or(0, |i| i + 1) as u32,
    );
    main.add(&account_row);
    prefs.add(&main);

    // Troca de tipo: categorias do tipo certo.
    {
        let (cats, opts, wrow) = (look.cats.clone(), options.clone(), category_row.downgrade());
        kind_row.connect_selected_notify(move |k| {
            let kind = if k.selected() == 1 { Kind::Income } else { Kind::Expense };
            let Some(row) = wrow.upgrade() else { return };
            let (fresh, new_opts) = category_combo(&cats, kind, None);
            row.set_model(fresh.model().as_ref());
            row.set_selected(0);
            *opts.borrow_mut() = new_opts;
        });
    }

    let hint = format!("O lançamento entra no mês da data. Sem data, fica em {}.", fin::month_label(&original.month));
    let when = widgets::group("Quando", Some(hint.as_str()));
    let (date_row, date) = date_row("Vencimento / data", original.due_date.clone(), true);
    when.add(&date_row);
    let paid = adw::SwitchRow::builder()
        .title(if original.kind == Kind::Income { "Já recebido" } else { "Já pago" })
        .active(original.paid)
        .build();
    {
        let wp = paid.downgrade();
        kind_row.connect_selected_notify(move |k| {
            if let Some(p) = wp.upgrade() {
                p.set_title(if k.selected() == 1 { "Já recebido" } else { "Já pago" });
            }
        });
    }
    when.add(&paid);
    prefs.add(&when);

    // Repetição (só ao criar).
    let repeat_row = adw::ComboRow::builder().title("Repetição").model(&gtk::StringList::new(&REPEATS)).build();
    let times = adw::SpinRow::with_range(2.0, 120.0, 1.0);
    times.set_title("Número de meses / parcelas");
    times.set_value(2.0);
    let total_toggle = adw::SwitchRow::builder()
        .title("O valor informado é o total")
        .subtitle("Divide entre as parcelas")
        .build();
    if !editing {
        let group = widgets::group("Repetição", Some("Recorrente se repete sozinho todo mês. Parcelado cria uma parcela por mês."));
        group.add(&repeat_row);
        group.add(&times);
        group.add(&total_toggle);
        let sync = {
            let (wt, wtt) = (times.downgrade(), total_toggle.downgrade());
            move |selected: u32| {
                if let (Some(t), Some(tt)) = (wt.upgrade(), wtt.upgrade()) {
                    t.set_visible(selected >= 2);
                    tt.set_visible(selected == 3);
                }
            }
        };
        sync(0);
        repeat_row.connect_selected_notify(move |r| sync(r.selected()));
        prefs.add(&group);
    } else if original.is_recurring() || original.is_installment() {
        let text = if original.is_recurring() {
            match &original.recurrence_until {
                Some(until) => format!("Repete todo mês até {}.", fin::month_label(until)),
                None => "Repete todo mês, sem data para acabar.".to_string(),
            }
        } else {
            format!(
                "Parcela {} de {}.",
                original.installment_num.unwrap_or(1),
                original.installment_total.unwrap_or(1)
            )
        };
        let group = widgets::group("Repetição", Some(text.as_str()));
        if original.is_recurring() {
            let stop = button_row("Parar de repetir depois deste mês", Some("media-playback-stop-symbolic"));
            let (c, e) = (ctx.clone(), original.clone());
            stop.connect_activated(move |_| {
                let (c2, e2) = (c.clone(), e.clone());
                widgets::confirm(
                    &c,
                    "Parar de repetir?",
                    &format!(
                        "As ocorrências depois de {} que ainda não foram pagas serão apagadas.",
                        fin::month_label(&e.month)
                    ),
                    "Parar",
                    true,
                    move || {
                        let Some(group) = e2.recurrence_group.as_deref() else { return };
                        if c2.check(c2.db.fin_end_recurrence(group, &e2.month)).is_some() {
                            c2.toast("A série termina neste mês.");
                            c2.nav.pop();
                            c2.changed();
                        }
                    },
                );
            });
            group.add(&stop);
        }
        prefs.add(&group);
    }

    let more = widgets::group("Mais", None);
    let excluded = adw::SwitchRow::builder()
        .title("Fora dos totais")
        .subtitle("Dinheiro que só passa pela conta: mexe no saldo, não nas receitas/despesas")
        .active(original.excluded)
        .build();
    let note = adw::EntryRow::builder().title("Observação").build();
    note.set_text(&original.note);
    more.add(&excluded);
    more.add(&note);
    prefs.add(&more);

    if editing {
        let actions = widgets::group("", None);
        let delete = widgets::pill_button("Apagar lançamento", Some("destructive-action"));
        delete.set_halign(gtk::Align::Center);
        let (c, e) = (ctx.clone(), original.clone());
        delete.connect_clicked(move |_| delete_entry(&c, &e));
        actions.add(&delete);
        prefs.add(&actions);
    }

    let header = adw::HeaderBar::new();
    let save = gtk::Button::with_label("Salvar");
    save.add_css_class("suggested-action");
    header.pack_end(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&prefs));
    let page = adw::NavigationPage::builder()
        .title(if editing { "Editar lançamento" } else { "Novo lançamento" })
        .child(&toolbar)
        .build();

    let accounts = look.accounts.clone();
    let c = ctx.clone();
    save.connect_clicked(move |_| {
        let desc = description.text().trim().to_string();
        if desc.is_empty() {
            c.toast("Dê uma descrição ao lançamento.");
            return;
        }
        if amount.value() <= 0.0 {
            c.toast("Informe um valor maior que zero.");
            return;
        }
        let mut e = (*original).clone();
        e.kind = if kind_row.selected() == 1 { Kind::Income } else { Kind::Expense };
        e.description = desc;
        e.amount = amount.value();
        e.category_id = (category_row.selected() as usize).checked_sub(1).and_then(|i| options.borrow().get(i).map(|c| c.id));
        e.account_id = (account_row.selected() as usize).checked_sub(1).and_then(|i| accounts.get(i)).map(|a| a.id);
        e.due_date = date.borrow().clone();
        // O mês segue a data; sem data, fica no mês em que foi criado.
        if let Some(d) = e.due_date.as_deref().and_then(fin::parse_date) {
            if !editing || !(e.is_recurring() || e.is_installment()) {
                e.month = fin::month_of(d);
            }
        }
        if e.paid != paid.is_active() {
            e.paid = paid.is_active();
            e.paid_date = e.paid.then(today_key);
        }
        e.excluded = excluded.is_active();
        e.note = note.text().trim().to_string();

        if !editing {
            let n = times.value().round() as u32;
            let repeat = match repeat_row.selected() {
                1 => Repeat::Monthly(None),
                2 => Repeat::Monthly(Some(n)),
                3 => {
                    if total_toggle.is_active() {
                        e.amount = fin::split_installments(e.amount, n)[0];
                    }
                    Repeat::Installments(n)
                }
                _ => Repeat::Once,
            };
            if c.check(c.db.fin_add_entry(&e, repeat)).is_some() {
                c.changed();
                c.nav.pop();
            }
            return;
        }
        if e.is_recurring() || e.is_installment() {
            let d = adw::MessageDialog::new(
                Some(&c.window),
                Some("Aplicar a alteração a"),
                Some(if e.is_recurring() {
                    "Este lançamento se repete todo mês."
                } else {
                    "Este lançamento faz parte de um parcelamento."
                }),
            );
            d.add_responses(&[("cancel", "Cancelar"), ("this", "Só este"), ("following", "Este e os próximos")]);
            d.set_response_appearance("following", adw::ResponseAppearance::Suggested);
            d.set_default_response(Some("following"));
            d.set_close_response("cancel");
            let c2 = c.clone();
            d.connect_response(None, move |_, r| {
                let scope = match r {
                    "this" => Scope::This,
                    "following" => Scope::Following,
                    _ => return,
                };
                if c2.check(c2.db.fin_update_entry(&e, scope)).is_some() {
                    c2.changed();
                    c2.nav.pop();
                }
            });
            d.present();
        } else if c.check(c.db.fin_update_entry(&e, Scope::This)).is_some() {
            c.changed();
            c.nav.pop();
        }
    });
    page
}

fn delete_entry(ctx: &Shared, e: &Entry) {
    let id = e.id;
    if e.is_recurring() || e.is_installment() {
        let d = adw::MessageDialog::new(
            Some(&ctx.window),
            Some("Apagar lançamento?"),
            Some(if e.is_recurring() {
                "Apague só esta ocorrência, esta e as próximas, ou a série inteira."
            } else {
                "Apague só esta parcela, esta e as próximas, ou todas."
            }),
        );
        d.add_responses(&[("cancel", "Cancelar"), ("this", "Só este"), ("following", "Este e os próximos"), ("all", "Todos")]);
        d.set_response_appearance("all", adw::ResponseAppearance::Destructive);
        d.set_response_appearance("following", adw::ResponseAppearance::Destructive);
        d.set_close_response("cancel");
        let c = ctx.clone();
        d.connect_response(None, move |_, r| {
            let scope = match r {
                "this" => Scope::This,
                "following" => Scope::Following,
                "all" => Scope::All,
                _ => return,
            };
            if c.check(c.db.fin_delete_entry(id, scope)).is_some() {
                c.nav.pop();
                c.changed();
            }
        });
        d.present();
        return;
    }
    let c = ctx.clone();
    widgets::confirm(ctx, "Apagar lançamento?", "", "Apagar", true, move || {
        if c.check(c.db.fin_delete_entry(id, Scope::This)).is_some() {
            c.nav.pop();
            c.changed();
        }
    });
}

/// Transferência: só dá para ver e apagar (as duas pernas juntas).
fn transfer_page(ctx: &Shared, e: &Entry, look: &Lookup) -> adw::NavigationPage {
    let prefs = adw::PreferencesPage::new();
    let group = widgets::group("Transferência", Some("Para corrigir, apague e faça de novo."));
    let list = widgets::boxed_list();
    let (r, _) = widgets::value_row("Descrição", &e.description);
    list.append(&r);
    let (r, _) = widgets::value_row("Valor", &money(ctx, e.amount));
    list.append(&r);
    let (r, _) = widgets::value_row("Data", &e.effective_date().map(fin::date_short).unwrap_or_default());
    list.append(&r);
    let (r, _) = widgets::value_row("Conta", look.account(e.account_id).map_or("—", |a| a.name.as_str()));
    list.append(&r);
    group.add(&list);
    prefs.add(&group);
    let actions = widgets::group("", None);
    let delete = widgets::pill_button("Apagar transferência", Some("destructive-action"));
    delete.set_halign(gtk::Align::Center);
    let (c, id) = (ctx.clone(), e.id);
    delete.connect_clicked(move |_| {
        let c2 = c.clone();
        widgets::confirm(&c, "Apagar transferência?", "Saída e entrada serão apagadas.", "Apagar", true, move || {
            if c2.check(c2.db.fin_delete_entry(id, Scope::This)).is_some() {
                c2.nav.pop();
                c2.changed();
            }
        });
    });
    actions.add(&delete);
    prefs.add(&actions);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&prefs));
    adw::NavigationPage::builder().title("Transferência").child(&toolbar).build()
}
