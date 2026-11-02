//! Document (un onglet) : texte, chemin, encodage, fins de ligne.

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Enc {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Ansi,
}

impl Enc {
    pub const ALL: [Enc; 5] = [Enc::Utf8, Enc::Utf8Bom, Enc::Utf16Le, Enc::Utf16Be, Enc::Ansi];
    pub fn label(self) -> &'static str {
        match self {
            Enc::Utf8 => "UTF-8",
            Enc::Utf8Bom => "UTF-8 avec BOM",
            Enc::Utf16Le => "UTF-16 LE",
            Enc::Utf16Be => "UTF-16 BE",
            Enc::Ansi => "ANSI (Windows-1252)",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Eol {
    Crlf,
    Lf,
    Cr,
}

impl Eol {
    pub const ALL: [Eol; 3] = [Eol::Crlf, Eol::Lf, Eol::Cr];
    pub fn label(self) -> &'static str {
        match self {
            Eol::Crlf => "Windows (CRLF)",
            Eol::Lf => "Unix (LF)",
            Eol::Cr => "Macintosh (CR)",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            Eol::Crlf => "CRLF",
            Eol::Lf => "LF",
            Eol::Cr => "CR",
        }
    }
    fn seq(self) -> &'static str {
        match self {
            Eol::Crlf => "\r\n",
            Eol::Lf => "\n",
            Eol::Cr => "\r",
        }
    }
    pub fn platform_default() -> Eol {
        if cfg!(windows) { Eol::Crlf } else { Eol::Lf }
    }
}

/// Décode des octets : détecte BOM / UTF-8 / sinon ANSI. Le texte retourné utilise `\n`.
pub fn decode(bytes: &[u8]) -> (String, Enc, Eol) {
    let (raw, enc): (String, Enc) = if let Some(b) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        (String::from_utf8_lossy(b).into_owned(), Enc::Utf8Bom)
    } else if let Some(b) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (encoding_rs::UTF_16LE.decode_without_bom_handling(b).0.into_owned(), Enc::Utf16Le)
    } else if let Some(b) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (encoding_rs::UTF_16BE.decode_without_bom_handling(b).0.into_owned(), Enc::Utf16Be)
    } else {
        match std::str::from_utf8(bytes) {
            Ok(s) => (s.to_string(), Enc::Utf8),
            Err(_) => (encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes).0.into_owned(), Enc::Ansi),
        }
    };
    let crlf = raw.matches("\r\n").count();
    let lf = raw.matches('\n').count() - crlf;
    let cr = raw.matches('\r').count() - crlf;
    let eol = if crlf == 0 && lf == 0 && cr == 0 {
        Eol::platform_default()
    } else if crlf >= lf && crlf >= cr {
        Eol::Crlf
    } else if lf >= cr {
        Eol::Lf
    } else {
        Eol::Cr
    };
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    (text, enc, eol)
}

/// Encode le texte ; retourne aussi `true` si des caractères n'ont pas pu être représentés.
pub fn encode(text: &str, enc: Enc, eol: Eol) -> (Vec<u8>, bool) {
    let s = if eol == Eol::Lf { text.to_string() } else { text.replace('\n', eol.seq()) };
    match enc {
        Enc::Utf8 => (s.into_bytes(), false),
        Enc::Utf8Bom => {
            let mut v = vec![0xEF, 0xBB, 0xBF];
            v.extend(s.into_bytes());
            (v, false)
        }
        Enc::Utf16Le => {
            let mut v = vec![0xFF, 0xFE];
            for u in s.encode_utf16() {
                v.extend(u.to_le_bytes());
            }
            (v, false)
        }
        Enc::Utf16Be => {
            let mut v = vec![0xFE, 0xFF];
            for u in s.encode_utf16() {
                v.extend(u.to_be_bytes());
            }
            (v, false)
        }
        Enc::Ansi => {
            let (b, _, lossy) = encoding_rs::WINDOWS_1252.encode(&s);
            (b.into_owned(), lossy)
        }
    }
}

fn hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

pub struct Doc {
    pub id: u64,
    pub path: Option<PathBuf>,
    pub text: String,
    pub enc: Enc,
    pub eol: Eol,
    pub untitled_n: usize,
    saved_hash: u64,
    pub modified: bool,
}

impl Doc {
    pub fn new_empty(id: u64, untitled_n: usize) -> Self {
        Doc {
            id,
            path: None,
            text: String::new(),
            enc: Enc::Utf8,
            eol: Eol::platform_default(),
            untitled_n,
            saved_hash: hash(""),
            modified: false,
        }
    }

    pub fn open(id: u64, path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let (text, enc, eol) = decode(&bytes);
        Ok(Doc {
            id,
            path: Some(path.to_path_buf()),
            saved_hash: hash(&text),
            text,
            enc,
            eol,
            untitled_n: 0,
            modified: false,
        })
    }

    pub fn title(&self) -> String {
        match &self.path {
            Some(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            None => format!("Sans titre {}", self.untitled_n),
        }
    }

    pub fn refresh_modified(&mut self) {
        self.modified = hash(&self.text) != self.saved_hash;
    }

    pub fn mark_saved(&mut self) {
        self.saved_hash = hash(&self.text);
        self.modified = false;
    }

    /// Enregistre sur `path`. Si des caractères ne rentrent pas dans l'encodage ANSI, bascule en UTF-8.
    pub fn save_to(&mut self, path: &Path) -> std::io::Result<()> {
        let (mut bytes, lossy) = encode(&self.text, self.enc, self.eol);
        if lossy {
            self.enc = Enc::Utf8;
            bytes = encode(&self.text, self.enc, self.eol).0;
        }
        std::fs::write(path, bytes)?;
        self.path = Some(path.to_path_buf());
        self.mark_saved();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aller_retour() {
        for enc in Enc::ALL {
            for eol in Eol::ALL {
                let (b, _) = encode("héllo\nmonde", enc, eol);
                let (t, e2, eol2) = decode(&b);
                assert_eq!(t, "héllo\nmonde");
                assert_eq!(eol2, eol);
                if enc != Enc::Utf8 {
                    assert_eq!(e2, enc);
                }
            }
        }
    }

    #[test]
    fn ansi_detecte() {
        let (t, e, _) = decode(&[0x63, 0x61, 0x66, 0xE9]);
        assert_eq!((t.as_str(), e), ("café", Enc::Ansi));
    }
}
