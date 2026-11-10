//! Just enough CSV (RFC 4180) for mailbox import/export: quoted fields,
//! doubled quotes, CRLF or LF, and spreadsheet-formula neutralising on export.

/// Parse CSV text into rows of fields. Blank lines are skipped.
pub fn parse(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut rows = vec![];
    let mut row: Vec<String> = vec![];
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    let mut line = 1;
    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => in_quotes = false,
                '\n' => {
                    line += 1;
                    field.push(c);
                }
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => in_quotes = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                line += 1;
                row.push(std::mem::take(&mut field));
                if !(row.len() == 1 && row[0].trim().is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            _ => field.push(c),
        }
    }
    if in_quotes {
        return Err(format!("a quoted field is not closed (line {line})"));
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        if !(row.len() == 1 && row[0].trim().is_empty()) {
            rows.push(row);
        }
    }
    Ok(rows)
}

/// One CSV field, quoted when needed. A value a spreadsheet would run as a
/// formula (`=`, `+`, `-`, `@`, tab, CR first) gets a leading `'`.
pub fn field(v: &str) -> String {
    let v = if v.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{v}")
    } else {
        v.to_string()
    };
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v
    }
}

pub fn line(fields: &[&str]) -> String {
    let mut s = fields
        .iter()
        .map(|f| field(f))
        .collect::<Vec<_>>()
        .join(",");
    s.push_str("\r\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quotes_and_line_endings() {
        let rows = parse("\u{feff}address,name\r\nana@x.test,\"Ito, Ana\"\n\nren@x.test,\"Ren \"\"R\"\" Sato\"\n").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], vec!["ana@x.test", "Ito, Ana"]);
        assert_eq!(rows[2], vec!["ren@x.test", "Ren \"R\" Sato"]);
        assert_eq!(parse("a,b").unwrap(), vec![vec!["a", "b"]]);
        assert!(parse("a,\"b").is_err());
    }

    #[test]
    fn export_round_trips_and_neutralises_formulas() {
        let l = line(&["ana@x.test", "Ito, \"Ana\"", "=HYPERLINK(\"x\")", "100"]);
        assert_eq!(
            l,
            "ana@x.test,\"Ito, \"\"Ana\"\"\",\"'=HYPERLINK(\"\"x\"\")\",100\r\n"
        );
        let back = parse(&l).unwrap();
        assert_eq!(back[0][1], "Ito, \"Ana\"");
        assert_eq!(back[0][2], "'=HYPERLINK(\"x\")");
    }
}
