//! Núcleo do UnLoop: tudo que não depende de interface gráfica.
//!
//! - [`model`]: enums com os códigos do Quitzilla, ícones (emoji), cores e entidades.
//! - [`calc`]: regras puras (sequência, períodos, troféus, metas, saldos).
//! - [`format`]: formatação pt-BR de durações e valores.
//! - [`presets`]: metas padrão, sugestões de hábitos, moedas.
//! - [`db`]: SQLite com migrações por `PRAGMA user_version`.
//! - [`backup`]: export/import JSON (compatível com o backup do Quitzilla) e CSV.

pub mod backup;
pub mod calc;
pub mod db;
pub mod format;
pub mod model;
pub mod presets;
pub mod units;

/// Instante atual em epoch ms.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
