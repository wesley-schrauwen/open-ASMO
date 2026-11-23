//! Decoder for Arcanum `.dlg` dialogue files (`dlg/`, one per script: `scr/01990Inn_Bed_Use.scr`
//! has `dlg/01990Inn_Bed_Use.dlg`; 0x4cee60 swaps the folder and the extension).
//!
//! A file is a list of entries of seven `{...}` groups: the number, the text a male PC sees,
//! the text a female PC sees, then the intelligence needed, a test, the response and a result
//! (dialogue's; kept as text). Bytes outside braces are ignored, `//` comments too. Text is
//! Windows-1252.

use std::collections::BTreeMap;

/// One entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DialogLine {
    pub text: String,
    pub female_text: String,
    pub intelligence: String,
    pub test: String,
    pub response: String,
    pub result: String,
}

/// A dialogue file: its entries by number.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DialogFile {
    pub lines: BTreeMap<i32, DialogLine>,
}

impl DialogFile {
    /// Parses a `.dlg` file. Entries without a number, or cut short, are skipped; a repeated
    /// number replaces the earlier one.
    pub fn parse(data: &[u8]) -> Self {
        let groups = groups(data);
        let mut lines = BTreeMap::new();
        for entry in groups.chunks(7) {
            let Some(number) = entry.first().and_then(|n| n.trim().parse::<i32>().ok()) else { continue };
            let field = |i: usize| entry.get(i).cloned().unwrap_or_default();
            lines.insert(number, DialogLine { text: field(1), female_text: field(2), intelligence: field(3), test: field(4), response: field(5), result: field(6) });
        }
        Self { lines }
    }

    /// Line `number`'s text for a male (`female` false) or female PC; the male text when the
    /// female one is empty.
    pub fn text(&self, number: i32, female: bool) -> Option<&str> {
        let line = self.lines.get(&number)?;
        Some(if female && !line.female_text.is_empty() { &line.female_text } else { &line.text })
    }
}

/// The path of script file `script_path`'s dialogue (0x4cee60): `scr/X.scr` → `dlg/X.dlg`.
pub fn dialog_path(script_path: &str) -> Option<String> {
    let rest = script_path.get(3..)?;
    let stem = rest.get(..rest.len().checked_sub(3)?)?;
    Some(format!("dlg{stem}dlg"))
}

/// Every `{...}` group's contents, in order, skipping `//` comments outside braces.
fn groups(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        match data[i] {
            b'{' => {
                let end = data[i + 1..].iter().position(|&b| b == b'}').map_or(data.len(), |p| i + 1 + p);
                out.push(data[i + 1..end].iter().map(|&b| crate::cp1252::decode_byte(b)).collect::<String>().replace("\r\n", "\n"));
                i = end + 1;
            }
            b'/' if data.get(i + 1) == Some(&b'/') => {
                i = data[i..].iter().position(|&b| b == b'\n').map_or(data.len(), |p| i + p + 1);
            }
            _ => i += 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_have_seven_groups() {
        let data = b"// Inn bed\n{1}{You must pay again.}{You must pay again, madam.}{}{}{}{}\n{2}{You must pay.}{}{}{}{}{}\n";
        let file = DialogFile::parse(data);
        assert_eq!(file.lines.len(), 2);
        assert_eq!(file.text(1, false), Some("You must pay again."));
        assert_eq!(file.text(1, true), Some("You must pay again, madam."));
        assert_eq!(file.text(2, true), Some("You must pay."), "no female text: the male one");
        assert_eq!(file.text(3, false), None);
    }

    #[test]
    fn the_dialogue_sits_beside_its_script() {
        assert_eq!(dialog_path("scr/01990Inn_Bed_Use.scr").as_deref(), Some("dlg/01990Inn_Bed_Use.dlg"));
        assert_eq!(dialog_path("sc"), None);
    }
}
