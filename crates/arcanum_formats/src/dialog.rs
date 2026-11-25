//! Decoder for Arcanum `.dlg` dialogue files (`dlg/`, one per script: `scr/01990Inn_Bed_Use.scr`
//! has `dlg/01990Inn_Bed_Use.dlg`; 0x4cee60 swaps the folder and the extension).
//!
//! A file is a list of entries of seven `{...}` groups (loader 0x412e10, entry 0x417860); bytes
//! outside braces are ignored, `//` comments too. Text is Windows-1252. Entries are kept by
//! number (the engine sorts them, 0x417e00, and binary-searches, 0x416ab0).
//!
//! - **An NPC line** has an empty intelligence group: `{n}{male PC text}{female PC text}{}
//!   {voice}{}{result}`. The voice is the speech file's index (`sound/speech/<dialogue>/
//!   v<voice>_<m|f>.mp3`); the result runs when the line is shown.
//! - **A PC option** has one: `{n}{text}{gender}{intelligence}{test}{response}{result}`. Gender:
//!   empty for anyone, `1` male, anything else female. Intelligence: `n > 0` needs at least `n`,
//!   `n < 0` at most `-n` (the "dumb" options). The test must pass ([`tests`]); choosing the
//!   option runs the result ([`effects`]) and then follows the response.
//! - **Responses:** `n > 0` goes to line `n`, 0 ends, `n < 0` (not decoded) goes to line `-n`.
//! - The options after an NPC line are the PC options that follow it in number order, up to the
//!   next NPC line; at most [`MAX_OPTIONS`] are shown (0x414f50).
//!
//! **Tests and results** are runs of two-letter opcodes with numbers (0x4150d0 / 0x415ba0): the
//! scan looks for the next letter or `$`, takes it and the next byte as the opcode (any case),
//! reads a number after it (`atoi`), and for opcodes with two, the second after skipping spaces
//! and the first number's digits (0x4167c0). Anything else between is skipped.

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
            if entry.len() < 7 {
                continue;
            }
            let Some(number) = entry.first().and_then(|n| n.trim().parse::<i32>().ok()) else {
                continue;
            };
            let field = |i: usize| entry.get(i).cloned().unwrap_or_default();
            lines.insert(
                number,
                DialogLine {
                    text: field(1),
                    female_text: field(2),
                    intelligence: field(3),
                    test: field(4),
                    response: field(5),
                    result: field(6),
                },
            );
        }
        Self { lines }
    }

    /// Line `number`'s text for a male (`female` false) or female PC; the male text when the
    /// female one is empty.
    pub fn text(&self, number: i32, female: bool) -> Option<&str> {
        let line = self.lines.get(&number)?;
        Some(if female && !line.female_text.is_empty() {
            &line.female_text
        } else {
            &line.text
        })
    }
}

impl DialogLine {
    /// An NPC line: no intelligence group.
    pub fn is_npc(&self) -> bool {
        self.intelligence.trim().is_empty()
    }

    /// A PC option's intelligence (`atoi`): `n > 0` at least, `n < 0` at most `-n`.
    pub fn intelligence(&self) -> i32 {
        atoi(self.intelligence.as_bytes())
    }

    /// A PC option's gender: `None` for anyone, else whether it is for males (`1`).
    pub fn for_males(&self) -> Option<bool> {
        let g = self.female_text.trim();
        (!g.is_empty()).then(|| atoi(g.as_bytes()) == 1)
    }

    /// A PC option's response: the line it goes to (0 ends; below 0 not decoded).
    pub fn response(&self) -> i32 {
        atoi(self.response.as_bytes())
    }

    /// An NPC line's speech file index (0: none).
    pub fn voice(&self) -> i32 {
        atoi(self.test.as_bytes())
    }
}

/// Most options shown after an NPC line.
pub const MAX_OPTIONS: usize = 5;

/// The test opcodes (table 0x5a06bc), in the engine's order.
pub const TEST_OPCODES: [&str; 37] = [
    "ps", "ch", "pe", "al", "ma", "ta", "gv", "gf", "qu", "re", "$$", "in", "ha", "lf", "lc", "tr",
    "sk", "ru", "rq", "fo", "le", "qb", "me", "ni", "qa", "ra", "pa", "ss", "wa", "wt", "pv", "pf",
    "na", "ar", "rp", "ia", "sc",
];

/// The result opcodes (table 0x5a0750), in the engine's order.
pub const EFFECT_OPCODES: [&str; 35] = [
    "$$", "re", "qu", "fl", "co", "gv", "gf", "mm", "al", "in", "lf", "lc", "tr", "ru", "rq", "jo",
    "wa", "lv", "ss", "sc", "so", "uw", "pv", "pf", "xp", "nk", "rp", "np", "ce", "fp", "su", "or",
    "ii", "ri", "et",
];

/// Test opcodes that read two numbers.
const TWO_NUMBER_TESTS: [&str; 10] = ["gv", "gf", "qu", "lf", "lc", "tr", "sk", "qb", "qa", "sc"];
/// Result opcodes that read two numbers.
const TWO_NUMBER_EFFECTS: [&str; 11] = [
    "qu", "gv", "gf", "lf", "lc", "tr", "jo", "uw", "pv", "pf", "np",
];
/// Test opcodes that read two numbers but are not in [`TWO_NUMBER_TESTS`]: PC vars and flags.
const TWO_NUMBER_PC: [&str; 2] = ["pv", "pf"];

/// How a result changes a reaction or alignment: `+n`/`-n` add, `>n` at least, `<n` at most,
/// a bare `n` sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Set,
    Add,
    AtLeast,
    AtMost,
}

/// One opcode of a test or a result, with its numbers (the second is `a` again when the opcode
/// reads one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    /// Lower case.
    pub op: [u8; 2],
    pub a: i32,
    pub b: i32,
    /// For results' `re` and `al`.
    pub change: Change,
}

impl Token {
    pub fn op(&self) -> &str {
        std::str::from_utf8(&self.op).unwrap_or("??")
    }
}

/// A test or result that met an opcode the engine does not know: the tokens before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownOpcode {
    pub before: Vec<Token>,
    pub op: String,
}

/// A test's tokens (0x4150d0). An unknown opcode makes the whole test false: `Err`.
pub fn tests(text: &str) -> Result<Vec<Token>, UnknownOpcode> {
    scan(
        text,
        &TEST_OPCODES,
        |op| TWO_NUMBER_TESTS.contains(&op) || TWO_NUMBER_PC.contains(&op),
        false,
    )
}

/// A result's tokens (0x415ba0). An unknown opcode stops it: `Err` holds what ran before.
pub fn effects(text: &str) -> Result<Vec<Token>, UnknownOpcode> {
    scan(
        text,
        &EFFECT_OPCODES,
        |op| TWO_NUMBER_EFFECTS.contains(&op),
        true,
    )
}

fn scan(
    text: &str,
    known: &[&str],
    two: impl Fn(&str) -> bool,
    changes: bool,
) -> Result<Vec<Token>, UnknownOpcode> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !(b[i].is_ascii_alphabetic() || b[i] == b'$') {
            i += 1;
            continue;
        }
        let op = [
            b[i].to_ascii_lowercase(),
            b.get(i + 1).map_or(0, u8::to_ascii_lowercase),
        ];
        let name = std::str::from_utf8(&op).unwrap_or("");
        if !known.contains(&name) {
            return Err(UnknownOpcode {
                before: out,
                op: String::from_utf8_lossy(&op).into_owned(),
            });
        }
        let rest = &b[(i + 2).min(b.len())..];
        let mut change = Change::Set;
        let mut number = rest;
        if changes && (name == "re" || name == "al") {
            let at = rest
                .iter()
                .position(|c| !c.is_ascii_whitespace())
                .unwrap_or(rest.len());
            match rest.get(at) {
                Some(b'+') | Some(b'-') => change = Change::Add,
                Some(b'>') => (change, number) = (Change::AtLeast, &rest[at + 1..]),
                Some(b'<') => (change, number) = (Change::AtMost, &rest[at + 1..]),
                _ => {}
            }
        }
        let a = atoi(number);
        let b2 = if two(name) { second(rest) } else { a };
        out.push(Token {
            op,
            a,
            b: b2,
            change,
        });
        i += 2;
    }
    Ok(out)
}

/// C's `atoi`: spaces, a sign, digits.
pub fn atoi(b: &[u8]) -> i32 {
    let mut i = b
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .unwrap_or(b.len());
    let negative = b.get(i) == Some(&b'-');
    if matches!(b.get(i), Some(b'-') | Some(b'+')) {
        i += 1;
    }
    let mut n: i64 = 0;
    while let Some(d) = b.get(i).filter(|c| c.is_ascii_digit()) {
        n = (n * 10 + i64::from(d - b'0')).min(i64::from(i32::MAX) + 1);
        i += 1;
    }
    (if negative { -n } else { n }).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// The second number (0x4167c0): skip spaces, skip the first number's digits, `atoi`.
fn second(b: &[u8]) -> i32 {
    let mut i = b
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .unwrap_or(b.len());
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    atoi(&b[i..])
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
                let end = data[i + 1..]
                    .iter()
                    .position(|&b| b == b'}')
                    .map_or(data.len(), |p| i + 1 + p);
                out.push(
                    data[i + 1..end]
                        .iter()
                        .map(|&b| crate::cp1252::decode_byte(b))
                        .collect::<String>()
                        .replace("\r\n", "\n"),
                );
                i = end + 1;
            }
            b'/' if data.get(i + 1) == Some(&b'/') => {
                i = data[i..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(data.len(), |p| i + p + 1);
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
        assert_eq!(
            file.text(2, true),
            Some("You must pay."),
            "no female text: the male one"
        );
        assert_eq!(file.text(3, false), None);
    }

    #[test]
    fn npc_lines_have_no_intelligence_and_options_do() {
        let file = DialogFile::parse(b"{10}{Hello, @pcname@.}{Hello, lady.}{}{3}{}{lf 2 1}\n{11}{Hi.}{1}{5}{gf 12 1, $$ 50}{20}{$$ -50, re +3}\n{12}{Me hungry.}{}{-4}{}{0}{}");
        let npc = &file.lines[&10];
        assert!(npc.is_npc());
        assert_eq!(npc.voice(), 3);
        let hi = &file.lines[&11];
        assert!(!hi.is_npc());
        assert_eq!(
            (hi.for_males(), hi.intelligence(), hi.response()),
            (Some(true), 5, 20)
        );
        let dumb = &file.lines[&12];
        assert_eq!(
            (dumb.for_males(), dumb.intelligence(), dumb.response()),
            (None, -4, 0)
        );
    }

    #[test]
    fn tests_and_results_are_two_letter_opcodes_with_numbers() {
        let t = tests("gf 12 1, $$ 50, re -3").unwrap();
        assert_eq!(
            t.iter().map(|t| (t.op(), t.a, t.b)).collect::<Vec<_>>(),
            [("gf", 12, 1), ("$$", 50, 50), ("re", -3, -3)]
        );
        assert!(tests("").unwrap().is_empty());
        assert_eq!(
            tests("gf 1 1 mm 4").unwrap_err().op,
            "mm",
            "an unknown opcode: the test fails"
        );
        assert_eq!(tests("$2750").unwrap_err().op, "$2");
        // A negative first number: the second is read from the same place (0x4167c0).
        assert_eq!(tests("qu -5 2").unwrap()[0].b, -5);
        let e = effects("$$ -50, re +3, re >10, al <-2, lf 6 1").unwrap();
        assert_eq!(
            e.iter()
                .map(|t| (t.op(), t.a, t.b, t.change))
                .collect::<Vec<_>>(),
            [
                ("$$", -50, -50, Change::Set),
                ("re", 3, 3, Change::Add),
                ("re", 10, 10, Change::AtLeast),
                ("al", -2, -2, Change::AtMost),
                ("lf", 6, 1, Change::Set),
            ]
        );
        let stopped = effects("lf 1 1, ADD BLESSING").unwrap_err();
        assert_eq!(
            (stopped.before.len(), stopped.op.as_str()),
            (1, "ad"),
            "stops at the unknown opcode"
        );
    }

    #[test]
    fn the_dialogue_sits_beside_its_script() {
        assert_eq!(
            dialog_path("scr/01990Inn_Bed_Use.scr").as_deref(),
            Some("dlg/01990Inn_Bed_Use.dlg")
        );
        assert_eq!(dialog_path("sc"), None);
    }
}
