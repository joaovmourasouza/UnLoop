//! Formatação pt-BR (sem dependências de i18n): durações, números e moeda.

use crate::model::CounterStyle;
use crate::units::*;

/// Inteiro com separador de milhar ".".
pub fn group_int(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(ch);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// Número com `decimals` casas: "1.234,50".
pub fn format_decimal(value: f64, decimals: usize) -> String {
    let fixed = format!("{:.*}", decimals, value.abs());
    let (int_part, frac_part) = match fixed.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (fixed.clone(), None),
    };
    let mut out = group_int(int_part.parse::<i64>().unwrap_or(0));
    if let Some(f) = frac_part {
        out.push(',');
        out.push_str(&f);
    }
    let is_zero = out.chars().all(|c| c == '0' || c == ',' || c == '.');
    if value < 0.0 && !is_zero {
        format!("-{out}")
    } else {
        out
    }
}

/// Número sem zeros desnecessários: 12 → "12", 12.5 → "12,5".
pub fn format_number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        group_int(value.round() as i64)
    } else {
        format_decimal(value, 2)
            .trim_end_matches('0')
            .trim_end_matches(',')
            .to_string()
    }
}

fn currency_symbol(code: &str) -> (&str, usize) {
    match code {
        "BRL" => ("R$", 2),
        "USD" => ("US$", 2),
        "EUR" => ("€", 2),
        "GBP" => ("£", 2),
        "JPY" => ("¥", 0),
        "KRW" => ("₩", 0),
        "CLP" => ("CLP", 0),
        "ARS" => ("ARS", 2),
        "MXN" => ("MX$", 2),
        "CAD" => ("CA$", 2),
        "AUD" => ("AU$", 2),
        "INR" => ("₹", 2),
        "CNY" => ("CN¥", 2),
        _ => (code, 2),
    }
}

pub fn format_money(value: f64, currency: &str) -> String {
    let (symbol, decimals) = currency_symbol(currency);
    let number = format_decimal(value.abs(), decimals);
    if value < 0.0 && number.chars().any(|c| ('1'..='9').contains(&c)) {
        format!("-{symbol} {number}")
    } else {
        format!("{symbol} {number}")
    }
}

fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{} {}", group_int(n), if n == 1 { one } else { many })
}

/// Duração do contador conforme o estilo escolhido.
pub fn format_counter(ms: i64, style: CounterStyle) -> String {
    let t = ms.max(0);
    let days = t / MS_PER_DAY;
    let hours = (t % MS_PER_DAY) / MS_PER_HOUR;
    let minutes = (t % MS_PER_HOUR) / MS_PER_MINUTE;
    let seconds = (t % MS_PER_MINUTE) / MS_PER_SECOND;
    let clock = format!("{hours:02}:{minutes:02}:{seconds:02}");

    match style {
        CounterStyle::Full => {
            if days > 0 {
                format!("{days}d {clock}")
            } else {
                clock
            }
        }
        CounterStyle::FullLabeled => {
            let mut parts = Vec::new();
            if days > 0 {
                parts.push(plural(days, "dia", "dias"));
            }
            parts.push(format!("{hours} h"));
            parts.push(format!("{minutes} min"));
            parts.push(format!("{seconds} s"));
            parts.join(" ")
        }
        CounterStyle::DaysHoursMinutes => format!("{days}d {hours}h {minutes}min"),
        CounterStyle::DaysHours => format!("{days}d {hours}h"),
        CounterStyle::WeeksDays => format!(
            "{} {}",
            plural(t / MS_PER_WEEK, "semana", "semanas"),
            plural((t % MS_PER_WEEK) / MS_PER_DAY, "dia", "dias")
        ),
        CounterStyle::MonthsDays => format!(
            "{} {}",
            plural(t / MS_PER_MONTH, "mês", "meses"),
            plural((t % MS_PER_MONTH) / MS_PER_DAY, "dia", "dias")
        ),
        CounterStyle::YearsMonthsDays => {
            let rest = t % MS_PER_YEAR;
            format!("{}a {}m {}d", t / MS_PER_YEAR, rest / MS_PER_MONTH, (rest % MS_PER_MONTH) / MS_PER_DAY)
        }
        CounterStyle::Minutes => format!("{} min", group_int(t / MS_PER_MINUTE)),
        CounterStyle::Hours => format!("{} h", group_int(t / MS_PER_HOUR)),
        CounterStyle::Days => plural(days, "dia", "dias"),
        CounterStyle::Weeks => plural(t / MS_PER_WEEK, "semana", "semanas"),
        CounterStyle::Months => plural(t / MS_PER_MONTH, "mês", "meses"),
    }
}

/// Duração curta com as duas maiores unidades ("3d 4h", "45 min").
pub fn format_compact(ms: i64) -> String {
    let t = ms.max(0);
    if t >= 365 * MS_PER_DAY {
        return format!("{}a {}d", t / MS_PER_YEAR, (t % MS_PER_YEAR) / MS_PER_DAY);
    }
    let days = t / MS_PER_DAY;
    let hours = (t % MS_PER_DAY) / MS_PER_HOUR;
    let minutes = (t % MS_PER_HOUR) / MS_PER_MINUTE;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}min")
    } else {
        format!("{minutes} min")
    }
}

/// Converte texto digitado ("1.234,56", "12,5", "12.5") em número. Vazio = 0.
pub fn parse_number(text: &str) -> Option<f64> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if t.is_empty() {
        return Some(0.0);
    }
    let normalized = if t.contains(',') { t.replace('.', "").replace(',', ".") } else { t };
    normalized.parse::<f64>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: i64 = MS_PER_DAY + 2 * MS_PER_HOUR + 3 * MS_PER_MINUTE + 4 * MS_PER_SECOND;

    #[test]
    fn numeros() {
        assert_eq!(group_int(1_234_567), "1.234.567");
        assert_eq!(group_int(-1500), "-1.500");
        assert_eq!(group_int(999), "999");
        assert_eq!(format_decimal(1234.5, 2), "1.234,50");
        assert_eq!(format_decimal(-0.001, 2), "0,00");
        assert_eq!(format_number(12.0), "12");
        assert_eq!(format_number(12.5), "12,5");
        assert_eq!(format_money(1234.5, "BRL"), "R$ 1.234,50");
        assert_eq!(format_money(-3.0, "EUR"), "-€ 3,00");
        assert_eq!(format_money(1500.0, "JPY"), "¥ 1.500");
        assert_eq!(format_money(2.0, "XYZ"), "XYZ 2,00");
    }

    #[test]
    fn parse() {
        assert_eq!(parse_number("1.234,56"), Some(1234.56));
        assert_eq!(parse_number("12,5"), Some(12.5));
        assert_eq!(parse_number("12.5"), Some(12.5));
        assert_eq!(parse_number(""), Some(0.0));
        assert_eq!(parse_number("abc"), None);
    }

    #[test]
    fn contador() {
        assert_eq!(format_counter(SAMPLE, CounterStyle::Full), "1d 02:03:04");
        assert_eq!(format_counter(2 * MS_PER_HOUR, CounterStyle::Full), "02:00:00");
        assert_eq!(format_counter(SAMPLE, CounterStyle::FullLabeled), "1 dia 2 h 3 min 4 s");
        assert_eq!(format_counter(SAMPLE, CounterStyle::DaysHoursMinutes), "1d 2h 3min");
        assert_eq!(format_counter(SAMPLE, CounterStyle::DaysHours), "1d 2h");
        assert_eq!(format_counter(9 * MS_PER_DAY, CounterStyle::WeeksDays), "1 semana 2 dias");
        assert_eq!(format_counter(40 * MS_PER_DAY, CounterStyle::MonthsDays), "1 mês 9 dias");
        assert_eq!(format_counter(400 * MS_PER_DAY, CounterStyle::YearsMonthsDays), "1a 1m 4d");
        assert_eq!(format_counter(1500 * MS_PER_MINUTE, CounterStyle::Minutes), "1.500 min");
        assert_eq!(format_counter(SAMPLE, CounterStyle::Hours), "26 h");
        assert_eq!(format_counter(SAMPLE, CounterStyle::Days), "1 dia");
        assert_eq!(format_counter(3 * MS_PER_DAY, CounterStyle::Days), "3 dias");
        assert_eq!(format_counter(-5000, CounterStyle::Full), "00:00:00");
    }

    #[test]
    fn compacto() {
        assert_eq!(format_compact(3 * MS_PER_DAY + 4 * MS_PER_HOUR), "3d 4h");
        assert_eq!(format_compact(2 * MS_PER_HOUR + 5 * MS_PER_MINUTE), "2h 5min");
        assert_eq!(format_compact(45 * MS_PER_MINUTE), "45 min");
        assert_eq!(format_compact(400 * MS_PER_DAY), "1a 34d");
    }
}
