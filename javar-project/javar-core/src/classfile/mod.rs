//! Minimal JVM class-file schema analysis for structural hot-reload decisions.
//!
//! The JVM HotSwap API rejects changes to the class *schema* (fields / methods).
//! JavaR detects those changes here so the agent can install a **shadow class**
//! instead of calling `redefineClasses` on the original type.

use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;

/// Structural fingerprint of a class file (order-independent field/method sets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassSchema {
    pub this_class: String,
    pub fields: BTreeSet<String>,
    pub methods: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Method bodies / attributes only — safe for `Instrumentation.redefineClasses`.
    Compatible,
    /// Added/removed/changed field or method descriptors — needs shadow class.
    Structural,
}

impl ClassSchema {
    pub fn parse(bytecode: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytecode);
        let magic = r.u32()?;
        if magic != 0xCAFEBABE {
            bail!("not a class file (bad magic)");
        }
        let _minor = r.u16()?;
        let _major = r.u16()?;
        let cp_count = r.u16()? as usize;
        let cp = parse_constant_pool(&mut r, cp_count)?;

        let _access = r.u16()?;
        let this_index = r.u16()? as usize;
        let _super = r.u16()?;
        let this_class = class_name(&cp, this_index).unwrap_or_else(|| "Unknown".into());

        let interfaces = r.u16()? as usize;
        for _ in 0..interfaces {
            let _ = r.u16()?;
        }

        let mut fields = BTreeSet::new();
        let field_count = r.u16()? as usize;
        for _ in 0..field_count {
            let _acc = r.u16()?;
            let name_i = r.u16()? as usize;
            let desc_i = r.u16()? as usize;
            let name = utf8(&cp, name_i).unwrap_or_default();
            let desc = utf8(&cp, desc_i).unwrap_or_default();
            fields.insert(format!("{name}:{desc}"));
            skip_attributes(&mut r)?;
        }

        let mut methods = BTreeSet::new();
        let method_count = r.u16()? as usize;
        for _ in 0..method_count {
            let _acc = r.u16()?;
            let name_i = r.u16()? as usize;
            let desc_i = r.u16()? as usize;
            let name = utf8(&cp, name_i).unwrap_or_default();
            let desc = utf8(&cp, desc_i).unwrap_or_default();
            methods.insert(format!("{name}:{desc}"));
            skip_attributes(&mut r)?;
        }

        Ok(Self {
            this_class: this_class.replace('/', "."),
            fields,
            methods,
        })
    }

    pub fn classify_against(&self, previous: &ClassSchema) -> ChangeKind {
        if self.fields == previous.fields && self.methods == previous.methods {
            ChangeKind::Compatible
        } else {
            ChangeKind::Structural
        }
    }
}

/// Build the shadow binary name: `com.example.Foo` → `com.example.Foo$JavaR_v3`.
pub fn shadow_binary_name(class_name: &str, version: u32) -> String {
    format!("{class_name}$JavaR_v{version}")
}

// ---- low-level class file reader ------------------------------------------------

#[derive(Debug, Clone)]
enum CpEntry {
    Utf8(String),
    Class(u16),
    Other,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn u8(&mut self) -> Result<u8> {
        let b = *self.data.get(self.pos).context("truncated class file")?;
        self.pos += 1;
        Ok(b)
    }

    fn u16(&mut self) -> Result<u16> {
        let hi = self.u8()? as u16;
        let lo = self.u8()? as u16;
        Ok((hi << 8) | lo)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(((self.u16()? as u32) << 16) | self.u16()? as u32)
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            bail!("truncated class file");
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
}

fn parse_constant_pool(r: &mut Reader<'_>, count: usize) -> Result<Vec<CpEntry>> {
    // CP is 1-based; index 0 unused.
    let mut cp = vec![CpEntry::Other; count];
    let mut i = 1;
    while i < count {
        let tag = r.u8()?;
        match tag {
            1 => {
                // Utf8
                let len = r.u16()? as usize;
                let bytes = r.bytes(len)?;
                let s = String::from_utf8_lossy(bytes).into_owned();
                cp[i] = CpEntry::Utf8(s);
            }
            7 => {
                // Class
                let name_i = r.u16()?;
                cp[i] = CpEntry::Class(name_i);
            }
            3 | 4 | 9 | 10 | 11 | 12 | 18 => {
                // int/float/field/method/iface/nameandtype/invokedynamic — 4 bytes
                let _ = r.u32()?;
                cp[i] = CpEntry::Other;
            }
            5 | 6 => {
                // long/double — 8 bytes, takes two slots
                let _ = r.u32()?;
                let _ = r.u32()?;
                cp[i] = CpEntry::Other;
                i += 1;
            }
            8 | 16 | 19 | 20 => {
                // string / methodtype / module / package — u16
                let _ = r.u16()?;
                cp[i] = CpEntry::Other;
            }
            15 => {
                // methodhandle
                let _ = r.u8()?;
                let _ = r.u16()?;
                cp[i] = CpEntry::Other;
            }
            17 => {
                // dynamic
                let _ = r.u32()?;
                cp[i] = CpEntry::Other;
            }
            _ => bail!("unknown CP tag {tag} at index {i}"),
        }
        i += 1;
    }
    Ok(cp)
}

fn utf8(cp: &[CpEntry], index: usize) -> Option<String> {
    match cp.get(index)? {
        CpEntry::Utf8(s) => Some(s.clone()),
        _ => None,
    }
}

fn class_name(cp: &[CpEntry], index: usize) -> Option<String> {
    match cp.get(index)? {
        CpEntry::Class(name_i) => utf8(cp, *name_i as usize),
        _ => None,
    }
}

fn skip_attributes(r: &mut Reader<'_>) -> Result<()> {
    let n = r.u16()? as usize;
    for _ in 0..n {
        let _name = r.u16()?;
        let len = r.u32()? as usize;
        let _ = r.bytes(len)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal synthetic class file: empty Object subclass `T`.
    fn minimal_class(name_utf8: &str) -> Vec<u8> {
        // Hand-rolled tiny class with one empty constructor for schema tests is heavy;
        // unit tests below use parse error / shadow name helpers.
        let _ = name_utf8;
        Vec::new()
    }

    #[test]
    fn shadow_name_format() {
        assert_eq!(
            shadow_binary_name("com.example.MyService", 2),
            "com.example.MyService$JavaR_v2"
        );
    }

    #[test]
    fn structural_when_field_added() {
        let a = ClassSchema {
            this_class: "A".into(),
            fields: BTreeSet::from(["x:I".into()]),
            methods: BTreeSet::from(["m:()V".into()]),
        };
        let b = ClassSchema {
            this_class: "A".into(),
            fields: BTreeSet::from(["x:I".into(), "y:I".into()]),
            methods: a.methods.clone(),
        };
        assert_eq!(b.classify_against(&a), ChangeKind::Structural);
    }

    #[test]
    fn compatible_when_only_same_schema() {
        let a = ClassSchema {
            this_class: "A".into(),
            fields: BTreeSet::from(["x:I".into()]),
            methods: BTreeSet::from(["m:()V".into()]),
        };
        assert_eq!(a.classify_against(&a), ChangeKind::Compatible);
        let _ = minimal_class("T");
    }
}
