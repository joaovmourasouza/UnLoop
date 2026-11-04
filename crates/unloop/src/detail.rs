//! Detalhe de um hábito: 7 abas num AdwViewStack.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use unloop_core::calc::{
    goal_reached, goal_remaining, goal_target, sort_goals_by_target, time_to_afford, HabitBundle, HabitSnapshot,
};
use unloop_core::db::TextTable;
use unloop_core::format::{format_compact, format_money};
use unloop_core::model::{color, DiaryEntry, Goal, Reward, RuleKind, TextItem, TimeInvestment, Trophy};
use unloop_core::presets::rule_suggestions;
use unloop_core::units::{MS_PER_DAY, MS_PER_MINUTE};

use crate::app::Shared;
use crate::widgets::{self, esc};
use crate::{entry, habit_form};

struct Tabs {
    overview: gtk::Box,
    rules: gtk::Box,
    notes: gtk::Box,
    diary: gtk::Box,
    goals: gtk::Box,
    stats: gtk::Box,
    trophies: gtk::Box,
    incentives: gtk::Box,
    motivation: gtk::Box,
    /// 0 = todas, 1 = recaídas, 2 = permitidos, 3 = com notas
    diary_filter: Cell<u32>,
}

pub fn page(ctx: &Shared, habit_id: i64) -> adw::NavigationPage {
    let stack = adw::ViewStack::new();
    let make = |name: &str, title: &str, icon: &str| -> gtk::Box {
        let content = widgets::content_box();
        let page = stack.add_titled(&widgets::scroller(&content), Some(name), title);
        page.set_icon_name(Some(icon));
        content
    };
    let tabs = Rc::new(Tabs {
        overview: make("overview", "Visão geral", "alarm-symbolic"),
        rules: make("rules", "Regras", "object-select-symbolic"),
        notes: make("notes", "Notas", "accessories-text-editor-symbolic"),
        diary: make("diary", "Diário", "document-edit-symbolic"),
        goals: make("goals", "Metas", "emblem-ok-symbolic"),
        stats: make("stats", "Estatísticas", "view-list-symbolic"),
        trophies: make("trophies", "Troféus", "starred-symbolic"),
        incentives: make("incentives", "Incentivos", "emblem-favorite-symbolic"),
        motivation: make("motivation", "Motivação", "weather-clear-symbolic"),
        diary_filter: Cell::new(0),
    });

    let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));

    let edit = widgets::icon_button("document-edit-symbolic", "Editar hábito");
    header.pack_end(&build_more_menu(ctx, habit_id));
    header.pack_end(&edit);

    let switcher_bar = adw::ViewSwitcherBar::builder().stack(&stack).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.add_bottom_bar(&switcher_bar);
    toolbar.set_content(Some(&stack));

    // Janela estreita: abas vão para a barra inferior.
    let bin = adw::BreakpointBin::builder()
        .width_request(360)
        .height_request(300)
        .child(&toolbar)
        .build();
    if let Ok(condition) = adw::BreakpointCondition::parse("max-width: 1000sp") {
        let breakpoint = adw::Breakpoint::new(condition);
        breakpoint.add_setter(&switcher, "visible", Some(&false.to_value()));
        breakpoint.add_setter(&switcher_bar, "reveal", Some(&true.to_value()));
        bin.add_breakpoint(breakpoint);
    }

    let page = adw::NavigationPage::builder().title("Hábito").child(&bin).build();

    let c = ctx.clone();
    edit.connect_clicked(move |_| {
        if let Some(h) = c.check(c.db.habit(habit_id)).flatten() {
            c.nav.push(&habit_form::page(&c, Some(h)));
        }
    });

    rebuild(ctx, habit_id, &tabs, &page);
    let (c, t, wp) = (ctx.clone(), tabs.clone(), page.downgrade());
    ctx.on_refresh(move || match wp.upgrade() {
        Some(p) => rebuild(&c, habit_id, &t, &p),
        None => false,
    });
    page
}

pub(crate) fn build_more_menu(ctx: &Shared, habit_id: i64) -> gtk::MenuButton {
    let popover = gtk::Popover::new();
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let entries: [(&str, i64); 3] = [("Mover para cima", -1), ("Mover para baixo", 1), ("Apagar hábito", 0)];
    for (text, delta) in entries {
        let button = gtk::Button::with_label(text);
        button.add_css_class("flat");
        if let Some(l) = button.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
        let (c, p) = (ctx.clone(), popover.downgrade());
        button.connect_clicked(move |_| {
            if let Some(p) = p.upgrade() {
                p.popdown();
            }
            if delta != 0 {
                if c.check(c.db.move_habit(habit_id, delta)).is_some() {
                    c.changed();
                }
                return;
            }
            let c2 = c.clone();
            widgets::confirm(
                &c,
                "Apagar este hábito?",
                "O hábito e todos os dados dele (diário, metas, troféus, recompensas, investimentos e motivos) serão apagados permanentemente.",
                "Apagar",
                true,
                move || {
                    if c2.check(c2.db.delete_habit(habit_id)).is_some() {
                        c2.nav.pop();
                        c2.changed();
                        c2.toast("Hábito apagado.");
                    }
                },
            );
        });
        menu.append(&button);
    }
    popover.set_child(Some(&menu));
    gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .popover(&popover)
        .tooltip_text("Mais opções")
        .build()
}

/// Reconstrói todas as abas. Retorna `false` se o hábito não existe mais.
fn rebuild(ctx: &Shared, habit_id: i64, tabs: &Rc<Tabs>, page: &adw::NavigationPage) -> bool {
    let Some(bundle) = ctx.check(ctx.db.bundle(habit_id)).flatten() else { return false };
    page.set_title(&bundle.habit.name);
    let snap = bundle.snapshot(unloop_core::now_ms());
    for b in [
        &tabs.overview,
        &tabs.rules,
        &tabs.notes,
        &tabs.diary,
        &tabs.goals,
        &tabs.stats,
        &tabs.trophies,
        &tabs.incentives,
        &tabs.motivation,
    ] {
        widgets::clear_box(b);
    }
    overview(ctx, &bundle, &tabs.overview);
    rules_tab(ctx, &bundle, &tabs.rules);
    crate::notes::tab(ctx, habit_id, &tabs.notes);
    diary(ctx, &bundle, tabs);
    goals(ctx, &bundle, &snap, &tabs.goals);
    stats(ctx, &bundle, &snap, &tabs.stats);
    trophies(&snap, &tabs.trophies);
    incentives(ctx, &bundle, &snap, &tabs.incentives);
    motivation(ctx, &bundle, &tabs.motivation);
    true
}

// ------------------------------------------------------------- Visão geral

fn overview(ctx: &Shared, bundle: &HabitBundle, b: &gtk::Box) {
    let h = &bundle.habit;
    b.append(&widgets::avatar(h.icon, h.color, true));

    let caption = widgets::label("", &["dim-label"]);
    let counter = widgets::label("", &["counter"]);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("thick");
    progress.add_css_class(&format!("bar-color-{}", color(h.color).code));
    let goal = widgets::label("", &[]);
    let since = widgets::label("", &["caption", "dim-label"]);
    b.append(&caption);
    b.append(&counter);
    b.append(&progress);
    b.append(&goal);
    b.append(&since);

    if !bundle.affirmations.is_empty() {
        let index = (unloop_core::now_ms() / MS_PER_DAY) as usize % bundle.affirmations.len();
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("card");
        let text = widgets::label(&format!("“{}”", bundle.affirmations[index].text), &["affirmation"]);
        text.set_margin_top(18);
        text.set_margin_bottom(18);
        text.set_margin_start(18);
        text.set_margin_end(18);
        card.append(&text);
        b.append(&card);
    }

    let group = widgets::group("Nesta sequência", None);
    let (money_row, money) = widgets::value_row("Dinheiro economizado", "");
    let (time_row, time) = widgets::value_row("Tempo recuperado", "");
    group.add(&money_row);
    group.add(&time_row);
    b.append(&group);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    buttons.set_halign(gtk::Align::Center);
    let relapse = widgets::pill_button("Registrar recaída", Some("destructive-action"));
    let note = widgets::pill_button("Anotar no diário", None);
    buttons.append(&relapse);
    buttons.append(&note);
    b.append(&buttons);

    let id = h.id;
    let c = ctx.clone();
    relapse.connect_clicked(move |_| c.nav.push(&entry::page(&c, id, None, true)));
    let c = ctx.clone();
    note.connect_clicked(move |_| c.nav.push(&entry::page(&c, id, None, false)));

    let currency = ctx.currency();
    let bundle = bundle.clone();
    let weak = (caption.downgrade(), counter.downgrade(), progress.downgrade(), goal.downgrade(), since.downgrade());
    let (wm, wt) = (money.downgrade(), time.downgrade());
    ctx.live(move |now| {
        let (Some(caption), Some(counter), Some(progress), Some(goal), Some(since), Some(money), Some(time)) = (
            weak.0.upgrade(),
            weak.1.upgrade(),
            weak.2.upgrade(),
            weak.3.upgrade(),
            weak.4.upgrade(),
            wm.upgrade(),
            wt.upgrade(),
        ) else {
            return false;
        };
        let s = bundle.snapshot(now);
        caption.set_label(widgets::counter_caption(&bundle.habit, &s));
        counter.set_label(&widgets::counter_text(&bundle.habit, &s));
        progress.set_fraction(s.goal_progress());
        goal.set_label(&widgets::goal_text(&s));
        since.set_label(&format!("Sequência atual desde {}", widgets::fmt_datetime(s.start)));
        money.set_label(&format_money(s.balances.saved_since_start, &currency));
        time.set_label(&format_compact(s.balances.time_since_start_ms));
        true
    });
}

// ------------------------------------------------------------------ Regras

fn rules_tab(ctx: &Shared, bundle: &HabitBundle, b: &gtk::Box) {
    let habit_id = bundle.habit.id;
    let intro = widgets::label(
        "Defina o que mantém a sequência e o que a reinicia. Ao registrar algo no diário, \
         escolha a regra: o app decide sozinho se o contador reinicia.",
        &["dim-label"],
    );
    b.append(&intro);

    for kind in [RuleKind::Allowed, RuleKind::Forbidden] {
        let description = match kind {
            RuleKind::Allowed => "Não reinicia a sequência.",
            RuleKind::Forbidden => "Reinicia a sequência.",
        };
        let group = widgets::group(&format!("{} {}", kind.emoji(), kind.label()), Some(description));

        let add = widgets::icon_button("list-add-symbolic", "Adicionar regra");
        let c = ctx.clone();
        add.connect_clicked(move |_| {
            let c2 = c.clone();
            let (heading, placeholder) = match kind {
                RuleKind::Allowed => ("O que é permitido?", "Ex.: refrigerante sem açúcar"),
                RuleKind::Forbidden => ("O que não é permitido?", "Ex.: refrigerante comum"),
            };
            widgets::ask_text(&c, heading, "", placeholder, move |text| {
                if c2.check(c2.db.insert_rule(habit_id, kind, &text)).is_some() {
                    c2.changed();
                }
            });
        });
        group.set_header_suffix(Some(&add));

        let rules: Vec<_> = bundle.rules.iter().filter(|r| r.kind == kind).collect();
        if rules.is_empty() {
            let empty = adw::ActionRow::builder()
                .title("Nada definido ainda")
                .subtitle("Use o botão + para adicionar.")
                .build();
            empty.add_css_class("dim-label");
            group.add(&empty);
        } else {
            let list = widgets::boxed_list();
            for rule in rules {
                let used = bundle.rule_usage(rule.id);
                let row = adw::ActionRow::builder()
                    .title(esc(&rule.text))
                    .title_lines(0)
                    .activatable(true)
                    .build();
                if used > 0 {
                    row.set_subtitle(&format!("Registrado {used} vez(es) no diário"));
                }
                row.add_prefix(&gtk::Label::new(Some(kind.emoji())));

                let (c, id, text) = (ctx.clone(), rule.id, rule.text.clone());
                row.connect_activated(move |_| {
                    let c2 = c.clone();
                    widgets::ask_text(&c, "Editar regra", &text, "", move |new_text| {
                        if c2.check(c2.db.update_rule(id, &new_text)).is_some() {
                            c2.changed();
                        }
                    });
                });

                let delete = widgets::icon_button("user-trash-symbolic", "Apagar regra");
                let (c, id) = (ctx.clone(), rule.id);
                delete.connect_clicked(move |_| {
                    let c2 = c.clone();
                    widgets::confirm(
                        &c,
                        "Apagar regra?",
                        "As entradas do diário que citam esta regra continuam lá, só sem o vínculo.",
                        "Apagar",
                        true,
                        move || {
                            if c2.check(c2.db.delete_rule(id)).is_some() {
                                c2.changed();
                            }
                        },
                    );
                });
                row.add_suffix(&delete);
                list.append(&row);
            }
            group.add(&list);
        }
        b.append(&group);
    }

    // Sugestões ainda não adicionadas.
    let (allowed, forbidden) = rule_suggestions(bundle.habit.icon);
    let existing: Vec<String> = bundle.rules.iter().map(|r| r.text.to_lowercase()).collect();
    let pending: Vec<(RuleKind, &str)> = allowed
        .iter()
        .map(|t| (RuleKind::Allowed, *t))
        .chain(forbidden.iter().map(|t| (RuleKind::Forbidden, *t)))
        .filter(|(_, t)| !existing.contains(&t.to_lowercase()))
        .collect();
    if !pending.is_empty() {
        let group = widgets::group("Sugestões para este hábito", Some("Clique para adicionar. Ajuste ao que faz sentido para você."));
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(3)
            .min_children_per_line(1)
            .column_spacing(6)
            .row_spacing(6)
            .build();
        for (kind, text) in pending {
            let button = gtk::Button::with_label(&format!("{} {}", kind.emoji(), text));
            let c = ctx.clone();
            button.connect_clicked(move |_| {
                if c.check(c.db.insert_rule(habit_id, kind, text)).is_some() {
                    c.changed();
                }
            });
            flow.append(&button);
        }
        group.add(&flow);
        b.append(&group);
    }
}

// ------------------------------------------------------------------ Diário

fn diary(ctx: &Shared, bundle: &HabitBundle, tabs: &Rc<Tabs>) {
    let b = &tabs.diary;
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let filter = gtk::DropDown::from_strings(&["Todas", "Recaídas", "Permitidos", "Com notas"]);
    filter.set_selected(tabs.diary_filter.get());
    let (wt, c) = (Rc::downgrade(tabs), ctx.clone());
    filter.connect_selected_notify(move |dd| {
        if let Some(t) = wt.upgrade() {
            t.diary_filter.set(dd.selected());
        }
        // Reconstrói depois que o sinal termina (o DropDown é recriado no rebuild).
        let c = c.clone();
        gtk::glib::idle_add_local_once(move || c.changed());
    });
    let add = widgets::pill_button("Nova entrada", Some("suggested-action"));
    add.set_hexpand(true);
    add.set_halign(gtk::Align::End);
    let (id, c) = (bundle.habit.id, ctx.clone());
    add.connect_clicked(move |_| c.nav.push(&entry::page(&c, id, None, false)));
    bar.append(&filter);
    bar.append(&add);
    b.append(&bar);

    let mode = tabs.diary_filter.get();
    let entries: Vec<&DiaryEntry> = bundle
        .entries
        .iter()
        .filter(|e| match mode {
            1 => e.is_reset,
            2 => bundle.rule(e.rule_id).is_some_and(|r| r.kind == RuleKind::Allowed),
            3 => e.has_note(),
            _ => true,
        })
        .collect();

    if entries.is_empty() {
        let status = adw::StatusPage::builder()
            .icon_name("document-edit-symbolic")
            .title(if bundle.entries.is_empty() { "Diário vazio" } else { "Nada com este filtro" })
            .description("Registre recaídas, gastos e reflexões. Com o tempo, o diário revela seus gatilhos.")
            .build();
        b.append(&status);
        return;
    }

    let currency = ctx.currency();
    let list = widgets::boxed_list();
    for e in entries {
        let rule = bundle.rule(e.rule_id);
        let mut parts: Vec<String> = Vec::new();
        if e.is_reset {
            parts.push("Recaída".into());
        }
        if let Some(r) = rule {
            parts.push(format!("{} {}", r.kind.emoji(), r.text));
        }
        if e.money_spent > 0.0 {
            parts.push(format_money(e.money_spent, &currency));
        }
        if e.time_spent_ms > 0 {
            parts.push(format_compact(e.time_spent_ms));
        }
        if let Some(note) = e.note.as_deref().filter(|n| !n.trim().is_empty()) {
            parts.push(note.trim().chars().take(160).collect());
        }
        let row = adw::ActionRow::builder()
            .title(widgets::fmt_datetime(e.occurred_at))
            .subtitle(esc(&parts.join(" · ")))
            .subtitle_lines(3)
            .activatable(true)
            .build();
        let prefix = match rule {
            Some(r) if r.kind == RuleKind::Allowed => "✅",
            _ if e.is_reset => "🔁",
            _ => "📝",
        };
        row.add_prefix(&gtk::Label::new(Some(prefix)));

        let delete = widgets::icon_button("user-trash-symbolic", "Apagar");
        let (c, entry_id, was_reset) = (ctx.clone(), e.id, e.is_reset);
        delete.connect_clicked(move |_| {
            let c2 = c.clone();
            widgets::confirm(
                &c,
                "Apagar entrada?",
                if was_reset {
                    "Esta recaída será apagada e o contador será recalculado."
                } else {
                    "Esta entrada será apagada permanentemente."
                },
                "Apagar",
                true,
                move || {
                    if c2.check(c2.db.delete_entry(entry_id)).is_some() {
                        c2.changed();
                    }
                },
            );
        });
        row.add_suffix(&delete);

        let (c, owned) = (ctx.clone(), e.clone());
        row.connect_activated(move |_| c.nav.push(&entry::page(&c, owned.habit_id, Some(owned.clone()), owned.is_reset)));
        list.append(&row);
    }
    b.append(&list);
}

// ------------------------------------------------------------------- Metas

fn goal_dot(g: &Goal) -> gtk::Label {
    let dot = gtk::Label::new(None);
    dot.set_markup(&format!("<span foreground=\"#{}\" size=\"x-large\">●</span>", g.hex_color));
    dot
}

fn goals(ctx: &Shared, bundle: &HabitBundle, snap: &HabitSnapshot, b: &gtk::Box) {
    let (start, now, habit_id) = (snap.start, snap.now, bundle.habit.id);

    if let Some(g) = &snap.goal {
        let current = widgets::group("Meta atual", None);
        let row = adw::ActionRow::builder()
            .title(esc(&g.name))
            .subtitle(format!(
                "{}% · faltam {} · {}",
                (snap.goal_progress() * 100.0).floor(),
                format_compact(snap.goal_remaining()),
                widgets::fmt_datetime(snap.goal_target())
            ))
            .build();
        row.add_prefix(&goal_dot(g));
        if g.is_forced_current {
            let unpin = gtk::Button::with_label("Desafixar");
            unpin.set_valign(gtk::Align::Center);
            let c = ctx.clone();
            unpin.connect_clicked(move |_| {
                if c.check(c.db.set_forced_goal(habit_id, None)).is_some() {
                    c.changed();
                }
            });
            row.add_suffix(&unpin);
        }
        current.add(&row);
        b.append(&current);
    }

    let create = widgets::pill_button("Nova meta pessoal", None);
    create.set_halign(gtk::Align::Start);
    let (c, hex) = (ctx.clone(), color(bundle.habit.color).hex.to_string());
    create.connect_clicked(move |_| {
        let (c2, hex) = (c.clone(), hex.clone());
        widgets::ask_text(&c, "Meta pessoal", "", "Ex.: aniversário, viagem, formatura", move |name| {
            let (c3, hex) = (c2.clone(), hex.clone());
            let now = unloop_core::now_ms();
            widgets::pick_datetime(&c2, "Data da meta", now + 30 * MS_PER_DAY, Some(now + MS_PER_MINUTE), None, move |target| {
                let g = Goal::personal(habit_id, &name, target, &hex);
                if c3.check(c3.db.insert_goal(&g)).is_some() {
                    c3.changed();
                }
            });
        });
    });
    b.append(&create);

    let sorted = sort_goals_by_target(&bundle.goals, start);
    let upcoming: Vec<&Goal> = sorted.iter().filter(|g| !goal_reached(g, start, now)).collect();
    let completed: Vec<&Goal> = sorted.iter().rev().filter(|g| goal_reached(g, start, now)).collect();
    let current_id = snap.goal.as_ref().map(|g| g.id);

    let up_group = widgets::group(&format!("Próximas ({})", upcoming.len()), None);
    let up_list = widgets::boxed_list();
    for g in upcoming {
        let row = adw::ActionRow::builder()
            .title(esc(&g.name))
            .subtitle(format!(
                "Faltam {} · {}",
                format_compact(goal_remaining(g, start, now)),
                widgets::fmt_date(goal_target(g, start))
            ))
            .build();
        row.add_prefix(&goal_dot(g));
        if Some(g.id) != current_id {
            let pin = widgets::icon_button("view-pin-symbolic", "Definir como meta atual");
            let (c, gid) = (ctx.clone(), g.id);
            pin.connect_clicked(move |_| {
                if c.check(c.db.set_forced_goal(habit_id, Some(gid))).is_some() {
                    c.changed();
                }
            });
            row.add_suffix(&pin);
        }
        if g.is_personal {
            row.add_suffix(&delete_goal_button(ctx, g));
        }
        up_list.append(&row);
    }
    up_group.add(&up_list);
    b.append(&up_group);

    let done_group = widgets::group(&format!("Concluídas ({})", completed.len()), None);
    if completed.is_empty() {
        done_group.set_description(Some("Sua primeira meta está a caminho."));
    } else {
        let done_list = widgets::boxed_list();
        for g in completed {
            let row = adw::ActionRow::builder()
                .title(esc(&g.name))
                .subtitle(format!("Alcançada em {}", widgets::fmt_datetime(goal_target(g, start))))
                .build();
            row.add_prefix(&gtk::Label::new(Some("✅")));
            if g.is_personal {
                row.add_suffix(&delete_goal_button(ctx, g));
            }
            done_list.append(&row);
        }
        done_group.add(&done_list);
    }
    b.append(&done_group);
}

fn delete_goal_button(ctx: &Shared, g: &Goal) -> gtk::Button {
    let button = widgets::icon_button("user-trash-symbolic", "Apagar meta pessoal");
    let (c, gid, name) = (ctx.clone(), g.id, g.name.clone());
    button.connect_clicked(move |_| {
        let c2 = c.clone();
        widgets::confirm(&c, "Apagar meta?", &format!("\"{name}\" será apagada."), "Apagar", true, move || {
            if c2.check(c2.db.delete_personal_goal(gid)).is_some() {
                c2.changed();
            }
        });
    });
    button
}

// ------------------------------------------------------------ Estatísticas

fn stats(ctx: &Shared, bundle: &HabitBundle, snap: &HabitSnapshot, b: &gtk::Box) {
    let p = &snap.periods;
    let bal = &snap.balances;
    let currency = ctx.currency();
    let dur = |v: Option<i64>| v.map(format_compact).unwrap_or_else(|| "—".into());

    let progress = widgets::group("Progresso", None);
    let rows: Vec<(String, String)> = vec![
        ("Início da jornada".into(), widgets::fmt_date(bundle.habit.quit_at)),
        ("Recaídas".into(), p.count().to_string()),
        ("Maior período limpo".into(), format_compact(p.longest().unwrap_or(0).max(snap.elapsed()))),
        ("Menor período encerrado".into(), dur(p.shortest())),
        ("Média dos períodos encerrados".into(), dur(p.average())),
        ("Período anterior".into(), dur(p.previous())),
        ("Troféus".into(), format!("{} de {}", snap.trophies_achieved(), Trophy::ALL.len())),
    ];
    let (streak_row, streak_value) = widgets::value_row("Sequência atual", "");
    progress.add(&streak_row);
    for (title, value) in rows {
        progress.add(&widgets::value_row(&title, &value).0);
    }
    b.append(&progress);
    let (weak, owned) = (streak_value.downgrade(), bundle.clone());
    ctx.live(move |now| {
        let Some(label) = weak.upgrade() else { return false };
        label.set_label(&format_compact(owned.snapshot(now).elapsed()));
        true
    });

    if !p.periods.is_empty() {
        let chart = widgets::group("Períodos sem recaída", Some("Os últimos 24 períodos encerrados e o atual."));
        let shown = &p.periods[p.periods.len().saturating_sub(24)..];
        let first_number = p.periods.len() - shown.len() + 1;
        let mut values: Vec<(String, i64)> =
            shown.iter().enumerate().map(|(i, v)| (format!("#{}", first_number + i), *v)).collect();
        values.push(("Atual".into(), snap.elapsed()));
        let max = values.iter().map(|v| v.1).max().unwrap_or(1).max(1);
        for (title, value) in values {
            let row = adw::ActionRow::builder().title(title).build();
            let bar = gtk::ProgressBar::new();
            bar.set_fraction(value as f64 / max as f64);
            bar.set_valign(gtk::Align::Center);
            bar.set_width_request(240);
            bar.add_css_class(&format!("bar-color-{}", color(bundle.habit.color).code));
            let text = gtk::Label::new(Some(&format_compact(value)));
            text.set_width_chars(9);
            text.set_xalign(1.0);
            row.add_suffix(&bar);
            row.add_suffix(&text);
            chart.add(&row);
        }
        b.append(&chart);
    }

    let money = widgets::group("Dinheiro", None);
    for (title, value) in [
        ("Economia desde o início", bal.saved_total),
        ("Economia desde a última recaída", bal.saved_since_start),
        ("Gasto com o hábito (diário)", bal.spent_on_habit),
        ("Gasto em recompensas", bal.rewards_spent),
        ("Saldo disponível", bal.money_balance()),
    ] {
        money.add(&widgets::value_row(title, &format_money(value, &currency)).0);
    }
    b.append(&money);

    let time = widgets::group("Tempo", None);
    for (title, value) in [
        ("Recuperado desde o início", bal.time_total_ms),
        ("Recuperado desde a última recaída", bal.time_since_start_ms),
        ("Perdido com o hábito (diário)", bal.time_lost_ms),
        ("Investido em coisas úteis", bal.invested_ms),
        ("Saldo disponível", bal.time_balance_ms().max(0)),
    ] {
        time.add(&widgets::value_row(title, &format_compact(value)).0);
    }
    b.append(&time);
}

// ----------------------------------------------------------------- Troféus

fn trophies(snap: &HabitSnapshot, b: &gtk::Box) {
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(6)
        .column_spacing(12)
        .row_spacing(12)
        .build();
    for &t in Trophy::ALL {
        let at = snap.trophy_at(t);
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.add_css_class("card");
        card.add_css_class("trophy");
        if at.is_none() {
            card.add_css_class("locked");
        }
        let emoji = if t == Trophy::DecisionMade { "🤝" } else if at.is_some() { "🏆" } else { "🔒" };
        card.append(&widgets::label(emoji, &["emoji"]));
        card.append(&widgets::label(t.label(), &["heading"]));
        let status = match at {
            Some(ms) => format!("Conquistado em {}", widgets::fmt_date(ms)),
            None => format!("Faltam {}", format_compact(t.ms() - snap.elapsed())),
        };
        card.append(&widgets::label(&status, &["caption", "dim-label"]));
        flow.append(&card);
    }
    b.append(&flow);
}

// --------------------------------------------------------------- Incentivos

fn incentives(ctx: &Shared, bundle: &HabitBundle, snap: &HabitSnapshot, b: &gtk::Box) {
    let bal = snap.balances.clone();
    let currency = ctx.currency();
    let habit_id = bundle.habit.id;

    let balance = widgets::group("Saldo", Some("Economia líquida menos o que você já gastou em recompensas e investimentos."));
    balance.add(&widgets::value_row("Dinheiro disponível", &format_money(bal.money_balance(), &currency)).0);
    balance.add(&widgets::value_row("Tempo disponível", &format_compact(bal.time_balance_ms().max(0))).0);
    let projection = adw::ActionRow::builder()
        .title("Projeção de economia")
        .subtitle("Quanto você terá economizado numa data futura")
        .activatable(true)
        .build();
    projection.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    {
        let (c, bal, currency) = (ctx.clone(), bal.clone(), currency.clone());
        projection.connect_activated(move |_| {
            let (c2, bal, currency) = (c.clone(), bal.clone(), currency.clone());
            let now = unloop_core::now_ms();
            widgets::pick_datetime(&c, "Até quando?", now + 365 * MS_PER_DAY, Some(now), None, move |target| {
                let ahead = (target - unloop_core::now_ms()).max(0) as f64;
                let body = format!(
                    "Economia total: {}\nA partir de hoje: {}\nTempo recuperado a partir de hoje: {}",
                    format_money(bal.net_saved() + bal.money_per_ms * ahead, &currency),
                    format_money(bal.money_per_ms * ahead, &currency),
                    format_compact((bal.time_per_ms * ahead).round() as i64)
                );
                widgets::info(&c2, &format!("Até {}", widgets::fmt_date(target)), &body);
            });
        });
    }
    balance.add(&projection);
    b.append(&balance);

    // Recompensas
    let rewards_group = widgets::group("Recompensas", Some("Compre algo com o dinheiro que deixou de gastar."));
    let add_reward = widgets::icon_button("list-add-symbolic", "Nova recompensa");
    {
        let (c, currency) = (ctx.clone(), currency.clone());
        add_reward.connect_clicked(move |_| {
            let c2 = c.clone();
            widgets::ask_name_number(&c, "Nova recompensa", &format!("Preço ({currency})"), "", 0.0, 2, move |name, price| {
                if c2.check(c2.db.insert_reward(&Reward::new(habit_id, &name, price))).is_some() {
                    c2.changed();
                }
            });
        });
    }
    rewards_group.set_header_suffix(Some(&add_reward));
    let visible: Vec<&Reward> = bundle.rewards.iter().filter(|r| !r.archived).collect();
    if !visible.is_empty() {
        let list = widgets::boxed_list();
        for r in visible {
            list.append(&reward_row(ctx, r, &bal, &currency));
        }
        rewards_group.add(&list);
    }
    b.append(&rewards_group);

    // Investimentos
    let inv_group = widgets::group("Investimentos de tempo", Some("Use o tempo recuperado em algo que importa."));
    let add_inv = widgets::icon_button("list-add-symbolic", "Novo investimento");
    {
        let c = ctx.clone();
        add_inv.connect_clicked(move |_| {
            let c2 = c.clone();
            widgets::ask_name_number(&c, "Novo investimento de tempo", "Minutos por sessão", "", 30.0, 0, move |name, minutes| {
                let t = TimeInvestment::new(habit_id, &name, (minutes * MS_PER_MINUTE as f64).round() as i64);
                if c2.check(c2.db.insert_investment(&t)).is_some() {
                    c2.changed();
                }
            });
        });
    }
    inv_group.set_header_suffix(Some(&add_inv));
    let visible: Vec<&TimeInvestment> = bundle.investments.iter().filter(|t| !t.archived).collect();
    if !visible.is_empty() {
        let list = widgets::boxed_list();
        for t in visible {
            list.append(&investment_row(ctx, t, bal.time_balance_ms()));
        }
        inv_group.add(&list);
    }
    b.append(&inv_group);
}

fn reward_row(ctx: &Shared, r: &Reward, bal: &unloop_core::calc::Balances, currency: &str) -> adw::ActionRow {
    let subtitle = match r.purchased_at {
        Some(at) => format!("Comprada em {}", widgets::fmt_date(at)),
        None => match time_to_afford(r.price, bal.money_balance(), bal.money_per_ms) {
            Some(0) => "Disponível!".to_string(),
            Some(wait) => format!("Disponível em {}", format_compact(wait)),
            None => "Defina um gasto diário no hábito para acompanhar.".to_string(),
        },
    };
    let row = adw::ActionRow::builder()
        .title(esc(&format!("{} · {}", r.name, format_money(r.price, currency))))
        .subtitle(subtitle)
        .build();
    row.add_prefix(&gtk::Label::new(Some(if r.is_purchased() { "✅" } else { "🎁" })));

    if !r.is_purchased() {
        let buy = gtk::Button::with_label("Comprar");
        buy.set_valign(gtk::Align::Center);
        buy.set_sensitive(bal.money_balance() >= r.price);
        let (c, id, name, price, currency) = (ctx.clone(), r.id, r.name.clone(), r.price, currency.to_string());
        buy.connect_clicked(move |_| {
            let c2 = c.clone();
            widgets::confirm(
                &c,
                &format!("Comprar \"{name}\"?"),
                &format!("{} serão descontados do seu saldo.", format_money(price, &currency)),
                "Comprar",
                false,
                move || {
                    if c2.check(c2.db.purchase_reward(id, unloop_core::now_ms())).is_some() {
                        c2.changed();
                        c2.toast("Aproveite! Você merece.");
                    }
                },
            );
        });
        row.add_suffix(&buy);

        let edit = widgets::icon_button("document-edit-symbolic", "Editar");
        let (c, id, name, price) = (ctx.clone(), r.id, r.name.clone(), r.price);
        edit.connect_clicked(move |_| {
            let c2 = c.clone();
            widgets::ask_name_number(&c, "Editar recompensa", "Preço", &name, price, 2, move |n, p| {
                if c2.check(c2.db.update_reward(id, &n, p)).is_some() {
                    c2.changed();
                }
            });
        });
        row.add_suffix(&edit);
    }

    let remove = widgets::icon_button("user-trash-symbolic", "Remover");
    let (c, owned) = (ctx.clone(), r.clone());
    remove.connect_clicked(move |_| {
        let (c2, owned2) = (c.clone(), owned.clone());
        widgets::confirm(
            &c,
            "Remover recompensa?",
            if owned.is_purchased() {
                "Ela sai da lista, mas o valor continua descontado do saldo, como numa compra real."
            } else {
                "Esta recompensa será apagada."
            },
            "Remover",
            true,
            move || {
                if c2.check(c2.db.remove_reward(&owned2)).is_some() {
                    c2.changed();
                }
            },
        );
    });
    row.add_suffix(&remove);
    row
}

fn investment_row(ctx: &Shared, t: &TimeInvestment, time_balance: i64) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(esc(&t.name))
        .subtitle(format!(
            "Sessão de {} · investido {}",
            format_compact(t.session_ms),
            format_compact(t.invested_ms)
        ))
        .build();
    row.add_prefix(&gtk::Label::new(Some("📚")));

    let invest = gtk::Button::with_label("Investir");
    invest.set_valign(gtk::Align::Center);
    invest.set_sensitive(time_balance >= t.session_ms);
    let (c, id) = (ctx.clone(), t.id);
    invest.connect_clicked(move |_| {
        if c.check(c.db.invest(id)).is_some() {
            c.changed();
        }
    });
    row.add_suffix(&invest);

    let remove = widgets::icon_button("user-trash-symbolic", "Remover");
    let (c, owned) = (ctx.clone(), t.clone());
    remove.connect_clicked(move |_| {
        let (c2, owned2) = (c.clone(), owned.clone());
        widgets::confirm(
            &c,
            "Remover investimento?",
            if owned.invested_ms > 0 {
                "Ele sai da lista, mas o tempo já investido continua contando no saldo."
            } else {
                "Este investimento será apagado."
            },
            "Remover",
            true,
            move || {
                if c2.check(c2.db.remove_investment(&owned2)).is_some() {
                    c2.changed();
                }
            },
        );
    });
    row.add_suffix(&remove);
    row
}

// ---------------------------------------------------------------- Motivação

pub(crate) fn motivation(ctx: &Shared, bundle: &HabitBundle, b: &gtk::Box) {
    let sections: [(&str, &str, TextTable, &Vec<TextItem>); 2] = [
        (
            "Por que eu quero parar",
            "Nos momentos difíceis, lembrar dos seus motivos dá força para resistir.",
            TextTable::Reason,
            &bundle.reasons,
        ),
        (
            "Afirmações",
            "Frases curtas para repetir para si. Uma delas aparece por dia na visão geral.",
            TextTable::Affirmation,
            &bundle.affirmations,
        ),
    ];
    let habit_id = bundle.habit.id;

    for (title, description, table, items) in sections {
        let group = widgets::group(title, Some(description));
        let add = widgets::icon_button("list-add-symbolic", "Adicionar");
        let c = ctx.clone();
        add.connect_clicked(move |_| {
            let c2 = c.clone();
            widgets::ask_text(&c, "Adicionar", "", "", move |text| {
                if c2.check(c2.db.insert_text_item(table, habit_id, &text)).is_some() {
                    c2.changed();
                }
            });
        });
        group.set_header_suffix(Some(&add));

        if !items.is_empty() {
            let list = widgets::boxed_list();
            for item in items {
                let row = adw::ActionRow::builder().title(esc(&item.text)).title_lines(0).activatable(true).build();
                let (c, id, text) = (ctx.clone(), item.id, item.text.clone());
                row.connect_activated(move |_| {
                    let c2 = c.clone();
                    widgets::ask_text(&c, "Editar", &text, "", move |new_text| {
                        if c2.check(c2.db.update_text_item(table, id, &new_text)).is_some() {
                            c2.changed();
                        }
                    });
                });
                let delete = widgets::icon_button("user-trash-symbolic", "Apagar");
                let (c, id) = (ctx.clone(), item.id);
                delete.connect_clicked(move |_| {
                    let c2 = c.clone();
                    widgets::confirm(&c, "Apagar?", "Este item será apagado.", "Apagar", true, move || {
                        if c2.check(c2.db.delete_text_item(table, id)).is_some() {
                            c2.changed();
                        }
                    });
                });
                row.add_suffix(&delete);
                list.append(&row);
            }
            group.add(&list);
        }
        b.append(&group);
    }
}
