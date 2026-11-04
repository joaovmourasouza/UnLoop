//! Unidades de tempo em milissegundos. Todas as datas são epoch ms (UTC).

pub const MS_PER_SECOND: i64 = 1_000;
pub const MS_PER_MINUTE: i64 = 60 * MS_PER_SECOND;
pub const MS_PER_HOUR: i64 = 60 * MS_PER_MINUTE;
pub const MS_PER_DAY: i64 = 24 * MS_PER_HOUR;
pub const MS_PER_WEEK: i64 = 7 * MS_PER_DAY;
/// Mês médio (30,4375 dias).
pub const MS_PER_MONTH: i64 = 2_629_800_000;
/// Ano médio (365,25 dias).
pub const MS_PER_YEAR: i64 = 31_557_600_000;
