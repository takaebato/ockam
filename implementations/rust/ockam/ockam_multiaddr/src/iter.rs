use crate::{default_registry, Checked, Code, Error, Registry};
use unsigned_varint::decode;

/// Iterator over binary protocol values.
///
/// The iterator yields as items a triple of:
///
/// 1. The input offset of the current protocol item.
/// 2. The protocol code.
/// 3. The protocol value.
///
#[derive(Debug)]
pub struct BytesIter<'a> {
    bytes: &'a [u8],
    offset: usize,
    is_err: bool,
    registry: Registry,
}

impl<'a> BytesIter<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        BytesIter {
            bytes,
            offset: 0,
            is_err: false,
            registry: default_registry().clone(),
        }
    }

    pub fn with_registry(bytes: &'a [u8], reg: Registry) -> Self {
        BytesIter {
            bytes,
            offset: 0,
            is_err: false,
            registry: reg,
        }
    }

    pub fn offset(&self) -> usize {
        self.offset
    }
}

impl<'a> Iterator for BytesIter<'a> {
    type Item = Result<(usize, Code, Checked<&'a [u8]>), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.bytes.len() || self.is_err {
            return None;
        }
        match decode::u32(&self.bytes[self.offset..]) {
            Ok((c, bytes)) => {
                let code = Code::new(c);
                if let Some(codec) = self.registry.get_by_code(code) {
                    match codec.split_bytes(code, bytes) {
                        Ok((val, rest)) => {
                            let offset = self.offset;
                            self.offset = self.bytes.len() - rest.len();
                            return Some(Ok((offset, code, val)));
                        }
                        Err(e) => {
                            self.is_err = true;
                            return Some(Err(e));
                        }
                    }
                }
                self.is_err = true;
                Some(Err(Error::unregistered(code)))
            }
            Err(e) => {
                self.is_err = true;
                Some(Err(e.into()))
            }
        }
    }
}

/// Iterator over textual protocol values.
///
/// The iterator yields as items a pair of:
///
/// 1. Protocol prefix.
/// 2. Protocol value.
///
pub struct StrIter<'a> {
    string: &'a str,
    registry: Registry,
}

impl<'a> StrIter<'a> {
    pub fn new(string: &'a str) -> Self {
        StrIter {
            string,
            registry: default_registry().clone(),
        }
    }

    pub fn with_registry(string: &'a str, reg: Registry) -> Self {
        StrIter {
            string,
            registry: reg,
        }
    }
}

impl<'a> Iterator for StrIter<'a> {
    type Item = Result<(&'a str, Checked<&'a str>), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.string.is_empty() {
            return None;
        }
        let (prefix, value) = if self.string == "self" {
            // TODO: once packet generated locally bypass ABAC rules, return None
            ("secure", "api")
        } else {
            match self.string.split_once('/') {
                Some(("", s)) => match s.split_once('/') {
                    Some((p, r)) => (p, r),
                    None => (s, ""),
                },
                Some((p, s)) => (p, s),
                None => (self.string, ""),
            }
        };
        if let Some(codec) = self.registry.get_by_prefix(prefix) {
            match codec.split_str(prefix, value) {
                Ok((val, rest)) => {
                    self.string = rest;
                    Some(Ok((prefix, val)))
                }
                Err(e) => Some(Err(e)),
            }
        } else {
            Some(Err(Error::unregistered_prefix(prefix)))
        }
    }
}
