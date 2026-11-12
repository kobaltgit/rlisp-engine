// src/interp/save.rs — сохранение прогресса игр (этап 29).
// Таблица ↔ JSON-файл: строки, числа (Int/Float), булевы и вложенные
// таблицы. Зависимость-ноль: собственный мини-парсер. Порядок ключей
// HashMap не гарантирован — для сохранений это не важно.
use crate::interp::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

fn json_escape(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn is_number(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Float(_))
}

/// Таблица → JSON. Списки/процедуры — ошибка (сохраняемые данные простые).
pub fn table_to_json(t: &Rc<RefCell<HashMap<String, Value>>>) -> Result<String, String> {
    let mut out = String::from("{");
    let map = t.borrow();
    let mut first = true;
    for (k, v) in map.iter() {
        if first {
            first = false;
        } else {
            out.push(',');
        }
        json_escape(&mut out, k);
        out.push(':');
        json_write(&mut out, v)?;
    }
    out.push('}');
    Ok(out)
}

fn json_write(out: &mut String, v: &Value) -> Result<(), String> {
    match v {
        Value::Str(s) => Ok(json_escape(out, s)),
        Value::Int(i) => Ok(out.push_str(&i.to_string())),
        // Debug-формат f64 печатает «1.0» с точкой — парсер отличит от Int
        Value::Float(f) => Ok(out.push_str(&format!("{:?}", f))),
        Value::Bool(b) => Ok(out.push_str(if *b { "true" } else { "false" })),
        Value::Table(t) => Ok(out.push_str(&table_to_json(t)?)),
        other => Err(format!(
            "save-table: значение «{}» — не строка/число/буль/таблица",
            crate::interp::value::fmt_value(other)
        )),
    }
}

/// JSON → существующая таблица (ключи поверх, таблица не очищается).
pub fn json_to_table(text: &str, t: &Rc<RefCell<HashMap<String, Value>>>) -> Result<(), String> {
    let bytes: Vec<char> = text.chars().collect();
    let mut pos = 0usize;
    parse_object(&bytes, &mut pos, t)?;
    skip_ws(&bytes, &mut pos);
    if pos != bytes.len() {
        return Err(format!("load-table: мусор после }} на позиции {}", pos));
    }
    Ok(())
}

fn skip_ws(b: &[char], pos: &mut usize) {
    while *pos < b.len() && b[*pos].is_whitespace() {
        *pos += 1;
    }
}

fn expect(b: &[char], pos: &mut usize, c: char, what: &str) -> Result<(), String> {
    skip_ws(b, pos);
    if *pos < b.len() && b[*pos] == c {
        *pos += 1;
        Ok(())
    } else {
        Err(format!("load-table: ожидается «{}» ({})", c, what))
    }
}

fn parse_string(b: &[char], pos: &mut usize) -> Result<String, String> {
    expect(b, pos, '"', "строка")?;
    let mut s = String::new();
    while *pos < b.len() {
        let c = b[*pos];
        *pos += 1;
        match c {
            '"' => return Ok(s),
            '\\' => {
                if *pos >= b.len() {
                    return Err("load-table: обрыв escape".into());
                }
                let e = b[*pos];
                *pos += 1;
                match e {
                    '"' => s.push('"'),
                    '\\' => s.push('\\'),
                    'n' => s.push('\n'),
                    't' => s.push('\t'),
                    'r' => s.push('\r'),
                    'u' => {
                        if *pos + 4 > b.len() {
                            return Err("load-table: обрыв \\u".into());
                        }
                        let hex: String = b[*pos..*pos + 4].iter().collect();
                        *pos += 4;
                        let n = u32::from_str_radix(&hex, 16)
                            .map_err(|_| "load-table: плохой \\u".to_string())?;
                        s.push(char::from_u32(n).unwrap_or('\u{FFFD}'));
                    }
                    other => return Err(format!("load-table: escape \\{}", other)),
                }
            }
            c => s.push(c),
        }
    }
    Err("load-table: строка не закрыта".into())
}

fn parse_number(b: &[char], pos: &mut usize) -> Result<Value, String> {
    let start = *pos;
    if *pos < b.len() && (b[*pos] == '-' || b[*pos] == '+') {
        *pos += 1;
    }
    let mut is_float = false;
    while *pos < b.len()
        && (b[*pos].is_ascii_digit()
            || b[*pos] == '.'
            || b[*pos] == 'e'
            || b[*pos] == 'E'
            || b[*pos] == '-')
    {
        if !b[*pos].is_ascii_digit() && b[*pos] != '-' {
            is_float = true;
        }
        *pos += 1;
    }
    let tok: String = b[start..*pos].iter().collect();
    if tok.is_empty() || tok == "-" {
        return Err("load-table: число не найдено".into());
    }
    if is_float {
        tok.parse::<f64>()
            .map(Value::Float)
            .map_err(|_| format!("load-table: плохое число «{}»", tok))
    } else {
        tok.parse::<i64>()
            .map(Value::Int)
            .map_err(|_| format!("load-table: плохое целое «{}»", tok))
    }
}

fn parse_value(b: &[char], pos: &mut usize) -> Result<Value, String> {
    skip_ws(b, pos);
    if *pos >= b.len() {
        return Err("load-table: неожиданный конец".into());
    }
    match b[*pos] {
        '"' => Ok(Value::Str(parse_string(b, pos)?)),
        '{' => {
            let inner: Rc<RefCell<HashMap<String, Value>>> = Rc::new(RefCell::new(HashMap::new()));
            parse_object(b, pos, &inner)?;
            Ok(Value::Table(inner))
        }
        't' => {
            *pos += 1;
            for c in "rue".chars() {
                if *pos >= b.len() || b[*pos] != c {
                    return Err("load-table: плохой литерал true".into());
                }
                *pos += 1;
            }
            Ok(Value::Bool(true))
        }
        'f' => {
            *pos += 1;
            for c in "alse".chars() {
                if *pos >= b.len() || b[*pos] != c {
                    return Err("load-table: плохой литерал false".into());
                }
                *pos += 1;
            }
            Ok(Value::Bool(false))
        }
        c if c.is_ascii_digit() || c == '-' || c == '+' => parse_number(b, pos),
        other => Err(format!(
            "load-table: неожиданный символ «{}»",
            other.escape_default()
        )),
    }
}

fn parse_object(
    b: &[char],
    pos: &mut usize,
    t: &Rc<RefCell<HashMap<String, Value>>>,
) -> Result<(), String> {
    expect(b, pos, '{', "объект")?;
    loop {
        skip_ws(b, pos);
        if *pos < b.len() && b[*pos] == '}' {
            *pos += 1;
            return Ok(());
        }
        let key = parse_string(b, pos)?;
        expect(b, pos, ':', "после ключа")?;
        let val = parse_value(b, pos)?;
        t.borrow_mut().insert(key, val);
        skip_ws(b, pos);
        if *pos < b.len() && b[*pos] == ',' {
            *pos += 1;
            continue;
        }
        if *pos < b.len() && b[*pos] == '}' {
            *pos += 1;
            return Ok(());
        }
        return Err("load-table: ожидается «,» или «}»".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tbl(pairs: Vec<(&str, Value)>) -> Rc<RefCell<HashMap<String, Value>>> {
        let m: HashMap<String, Value> =
            pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        Rc::new(RefCell::new(m))
    }

    #[test]
    fn roundtrip_числа_строки_були_вложенность() {
        let src = tbl(vec![
            ("score", Value::Int(1250)),
            ("hp", Value::Float(87.5)),
            ("имя", Value::Str("ЁЖИК \"МЁРТВЫЙ\"\n".into())),
            ("alive", Value::Bool(true)),
            ("lost", Value::Bool(false)),
            ("opts", Value::Table(tbl(vec![("level", Value::Int(3))]))),
        ]);
        let json = table_to_json(&src).unwrap();
        let dst: Rc<RefCell<HashMap<String, Value>>> = Rc::new(RefCell::new(HashMap::new()));
        json_to_table(&json, &dst).unwrap();
        let m = dst.borrow();
        assert!(matches!(m.get("score"), Some(Value::Int(1250))));
        assert!(matches!(m.get("hp"), Some(Value::Float(f)) if *f == 87.5));
        assert!(matches!(m.get("имя"),
            Some(Value::Str(s)) if s == "ЁЖИК \"МЁРТВЫЙ\"\n"));
        assert!(matches!(m.get("alive"), Some(Value::Bool(true))));
        assert!(matches!(m.get("lost"), Some(Value::Bool(false))));
        match m.get("opts") {
            Some(Value::Table(t)) => {
                assert!(matches!(t.borrow().get("level"), Some(Value::Int(3))));
            }
            other => panic!(
                "opts не таблица: {}",
                other.map(fmt_dbg).unwrap_or_default()
            ),
        }
        // Int остаётся Int, Float остаётся Float (не 1250.0)
        assert!(matches!(m.get("score"), Some(Value::Int(_))));
        assert!(matches!(m.get("hp"), Some(Value::Float(_))));
    }

    fn fmt_dbg(v: &Value) -> String {
        crate::interp::value::fmt_value(v)
    }

    #[test]
    fn float_1_0_сохраняется_с_точкой() {
        let src = tbl(vec![("f", Value::Float(1.0))]);
        let json = table_to_json(&src).unwrap();
        assert!(json.contains("1.0"), "json: {}", json);
        let dst: Rc<RefCell<HashMap<String, Value>>> = Rc::new(RefCell::new(HashMap::new()));
        json_to_table(&json, &dst).unwrap();
        assert!(matches!(dst.borrow().get("f"), Some(Value::Float(_))));
    }

    #[test]
    fn список_и_процедура_ошибка() {
        let src = tbl(vec![("bad", Value::List(vec![Value::Int(1)]))]);
        assert!(table_to_json(&src).is_err());
    }

    #[test]
    fn битый_json_честная_ошибка() {
        let dst: Rc<RefCell<HashMap<String, Value>>> = Rc::new(RefCell::new(HashMap::new()));
        assert!(json_to_table("{\"a\":}", &dst).is_err());
        assert!(json_to_table("{\"a\":1", &dst).is_err());
        assert!(json_to_table("{\"a\":1} мусор", &dst).is_err());
        assert!(json_to_table("", &dst).is_err());
    }
}
