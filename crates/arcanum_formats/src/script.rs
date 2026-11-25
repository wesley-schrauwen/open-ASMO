//! Decoder for Arcanum `.scr` script files (Arcanum.exe: index 0x4cec10 / 0x4cece0 / 0x4cedc0,
//! loader 0x44c4e0, line access 0x44c1b0, runner 0x4449b0).
//!
//! A script is a list of lines, each "if CONDITION then ACTION else ACTION". The engine finds
//! scripts by number: the leading digits of each `scr/*.scr` file name (`01729Bed_Use.scr` is
//! script 1729; see [`ScriptIndex`]).
//!
//! Layout (little-endian; the file is `0x40 + n * 0x84` bytes):
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0x00 | 8 | ignored by the loader (zero, or stale memory from the editor) |
//! | 0x08 | 0x28 | description, NUL-terminated (the rest is garbage) |
//! | 0x30 | 4 | flags ([`RUN_ONCE`]; other bits not decoded) |
//! | 0x34 | 4 | `n`, the number of lines |
//! | 0x38 | 4 | capacity (the editor's; at least `n`) |
//! | 0x3c | 4 | a stale pointer |
//! | 0x40 | n x 0x84 | lines |
//!
//! A line is three blocks of 0x2c bytes: the condition, the action when it holds, the action
//! when it does not. A block is a type (`u32` at +0x00), eight operand tags (`i8` at
//! +0x04..+0x0b) and eight operand values (`i32` at +0x0c..+0x2b); the operands a type does not
//! use are garbage. A scalar operand's tag is an [`OperandKind`] (getter 0x44ba70); an object
//! operand's tag is an object set (0x44b4e0).

use std::collections::BTreeMap;
use std::fmt;

/// Bytes before the first line.
pub const HEADER_SIZE: usize = 0x40;
/// Bytes per line.
pub const LINE_SIZE: usize = 0x84;
/// Bytes per block (a condition or an action).
pub const BLOCK_SIZE: usize = 0x2c;
/// Script flag: the attachment is cleared after the script ends (0x4449b0).
pub const RUN_ONCE: u32 = 0x4;
/// Scripts numbered below this are looked up in their own table (0x4cece0).
pub const LOW_NUMBERS: u32 = 1000;
/// The folder holding the scripts.
pub const DIR: &str = "scr";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    /// Shorter than its header, or than its header says.
    Truncated { needed: usize, len: usize },
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, len } => write!(f, "truncated: {len} bytes, {needed} needed"),
        }
    }
}

impl std::error::Error for ScriptError {}

/// What an operand's tag says its value is (getter 0x44ba70, jump table 0x44bb34; setter
/// 0x44bb55, table 0x44bc3c). Checked against the tables: 1 and 4 call the global accessors
/// (0x444c80, 0x444cf0), 5 and 6 the PC's (0x444db0, 0x444e30).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandKind {
    /// Tag 0: byte `value` (0-3) of the attachment's four counters.
    Counter,
    /// Tag 1: global variable `value` (2000 of them, saved).
    GlobalVar,
    /// Tag 2: variable `value` local to this run of the script (10 of them).
    Local,
    /// Tag 3: the number itself.
    Number,
    /// Tag 4: global flag `value` (3200 of them, saved).
    GlobalFlag,
    /// Tag 5: a PC variable (field 0x111): the high 16 bits of `value` pick the object (an
    /// object set, the first of it), the low 16 the index. 0 for anything but a PC.
    PcVar,
    /// Tag 6: a PC flag (field 0x110), chosen as for [`Self::PcVar`].
    PcFlag,
    /// Any other tag: read as the number itself.
    Other(i8),
}

impl OperandKind {
    pub fn from_tag(tag: i8) -> Self {
        match tag {
            0 => Self::Counter,
            1 => Self::GlobalVar,
            2 => Self::Local,
            3 => Self::Number,
            4 => Self::GlobalFlag,
            5 => Self::PcVar,
            6 => Self::PcFlag,
            other => Self::Other(other),
        }
    }
}

/// One operand: its tag and value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Operand {
    pub tag: i8,
    pub value: i32,
}

impl Operand {
    pub fn kind(self) -> OperandKind {
        OperandKind::from_tag(self.tag)
    }
}

/// Operands per block.
pub const OPERANDS: usize = 8;

/// A condition or an action: its type and operands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Block {
    pub kind: u32,
    pub operands: [Operand; OPERANDS],
}

impl Block {
    fn parse(b: &[u8]) -> Self {
        let i32_at = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().expect("4 bytes"));
        let operands = std::array::from_fn(|i| Operand {
            tag: b[4 + i] as i8,
            value: i32_at(0x0c + 4 * i),
        });
        Self {
            kind: i32_at(0) as u32,
            operands,
        }
    }
}

/// "If `condition` then `then` else `otherwise`".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Line {
    pub condition: Block,
    pub then: Block,
    pub otherwise: Block,
}

/// A script file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script {
    pub description: String,
    pub flags: u32,
    pub lines: Vec<Line>,
}

impl Script {
    /// Parses a `.scr` file. Bytes past the last line are ignored.
    pub fn parse(data: &[u8]) -> Result<Self, ScriptError> {
        if data.len() < HEADER_SIZE {
            return Err(ScriptError::Truncated {
                needed: HEADER_SIZE,
                len: data.len(),
            });
        }
        let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().expect("4 bytes"));
        let count = u32_at(0x34) as usize;
        let needed = count
            .checked_mul(LINE_SIZE)
            .and_then(|n| n.checked_add(HEADER_SIZE))
            .unwrap_or(usize::MAX);
        if data.len() < needed {
            return Err(ScriptError::Truncated {
                needed,
                len: data.len(),
            });
        }
        let description = data[0x08..0x30]
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| crate::cp1252::decode_byte(b))
            .collect();
        let lines = data[HEADER_SIZE..needed]
            .as_chunks::<LINE_SIZE>()
            .0
            .iter()
            .map(|l| Line {
                condition: Block::parse(&l[..BLOCK_SIZE]),
                then: Block::parse(&l[BLOCK_SIZE..2 * BLOCK_SIZE]),
                otherwise: Block::parse(&l[2 * BLOCK_SIZE..]),
            })
            .collect();
        Ok(Self {
            description,
            flags: u32_at(0x30),
            lines,
        })
    }

    /// Whether the attachment is cleared once the script has run.
    pub fn runs_once(&self) -> bool {
        self.flags & RUN_ONCE != 0
    }
}

/// A script's number: the leading digits of its file name (`atoi`, 0x4cec10). `None` without
/// digits or for 0.
pub fn script_number(file_name: &str) -> Option<u32> {
    let digits: String = file_name.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|&n| n > 0)
}

/// Script numbers to file names (0x4cec10 / 0x4cece0): every `.scr` file in [`DIR`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptIndex {
    pub files: BTreeMap<u32, String>,
    /// File names whose number was taken already ("Multiple script files numbered %.5d");
    /// the first, in listing order, wins.
    pub duplicates: Vec<String>,
}

impl ScriptIndex {
    /// Indexes the file names of the scripts folder; names without the `.scr` extension or a
    /// number are skipped.
    pub fn new<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut index = Self::default();
        for name in names {
            let is_script = name
                .rsplit_once('.')
                .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("scr"));
            let Some(number) = script_number(name).filter(|_| is_script) else {
                continue;
            };
            match index.files.entry(number) {
                std::collections::btree_map::Entry::Occupied(_) => {
                    index.duplicates.push(name.to_string())
                }
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(name.to_string());
                }
            }
        }
        index
    }

    /// The path of script `number` (0x4cedc0: `scr\%s`).
    pub fn path(&self, number: u32) -> Option<String> {
        self.files.get(&number).map(|name| format!("{DIR}/{name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(kind: u32, ops: &[(i8, i32)]) -> Vec<u8> {
        let mut b = vec![0xcc; BLOCK_SIZE];
        b[..4].copy_from_slice(&kind.to_le_bytes());
        for (i, &(tag, value)) in ops.iter().enumerate() {
            b[4 + i] = tag as u8;
            b[0x0c + 4 * i..0x10 + 4 * i].copy_from_slice(&value.to_le_bytes());
        }
        b
    }

    fn file(description: &str, flags: u32, lines: &[[Vec<u8>; 3]]) -> Vec<u8> {
        let mut out = vec![0u8; HEADER_SIZE];
        out[8..8 + description.len()].copy_from_slice(description.as_bytes());
        out[0x30..0x34].copy_from_slice(&flags.to_le_bytes());
        out[0x34..0x38].copy_from_slice(&(lines.len() as u32).to_le_bytes());
        out[0x38..0x3c].copy_from_slice(&10u32.to_le_bytes());
        for line in lines {
            for b in line {
                out.extend_from_slice(b);
            }
        }
        out
    }

    #[test]
    fn lines_are_condition_then_else() {
        // As 01621Open_Bed_CM.scr: if global flag 2273 is 1, run the default (action 2), else
        // skip it (action 1).
        let data = file(
            "Search the bed",
            0,
            &[[block(4, &[(4, 2273), (3, 1)]), block(2, &[]), block(1, &[])]],
        );
        assert_eq!(data.len(), HEADER_SIZE + LINE_SIZE);
        let script = Script::parse(&data).unwrap();
        assert_eq!(script.description, "Search the bed");
        assert!(!script.runs_once());
        let line = script.lines[0];
        assert_eq!(line.condition.kind, 4);
        assert_eq!(
            line.condition.operands[0],
            Operand {
                tag: 4,
                value: 2273
            }
        );
        assert_eq!(line.condition.operands[0].kind(), OperandKind::GlobalFlag);
        assert_eq!(line.condition.operands[1].kind(), OperandKind::Number);
        assert_eq!((line.then.kind, line.otherwise.kind), (2, 1));
    }

    #[test]
    fn short_files_are_refused_and_empty_scripts_are_fine() {
        assert!(Script::parse(&[0; 0x3f]).is_err());
        let mut data = file("", RUN_ONCE, &[]);
        assert_eq!(
            Script::parse(&data).unwrap(),
            Script {
                description: String::new(),
                flags: RUN_ONCE,
                lines: vec![]
            }
        );
        data[0x34] = 1;
        assert_eq!(
            Script::parse(&data),
            Err(ScriptError::Truncated {
                needed: HEADER_SIZE + LINE_SIZE,
                len: HEADER_SIZE
            })
        );
    }

    #[test]
    fn scripts_are_found_by_their_leading_digits() {
        assert_eq!(script_number("01729Bed_Use.scr"), Some(1729));
        assert_eq!(script_number("00997EndSlides.scr"), Some(997));
        assert_eq!(
            (script_number("Bed.scr"), script_number("00000.scr")),
            (None, None)
        );
        let index = ScriptIndex::new([
            "01729Bed_Use.scr",
            "30017_TechBook_README.txt",
            "30017Book.SCR",
            "01729Other.scr",
            "notes.scr",
        ]);
        assert_eq!(index.path(1729).as_deref(), Some("scr/01729Bed_Use.scr"));
        assert_eq!(
            index.path(30017).as_deref(),
            Some("scr/30017Book.SCR"),
            "only .scr files"
        );
        assert_eq!(index.duplicates, ["01729Other.scr"]);
        assert_eq!(index.files.len(), 2);
    }
}
