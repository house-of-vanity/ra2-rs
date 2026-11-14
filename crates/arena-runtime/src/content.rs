//! Content access is separate from both format decoding and GPU resource creation.
use std::path::{Path, PathBuf};

pub struct TextureBytes {
    pub key: String,
    pub extension: String,
    pub bytes: Vec<u8>,
}

/// An archive, mod package, or in-memory provider can implement this without changing rendering.
pub trait TextureSource {
    fn read(&self, name: &str) -> Result<TextureBytes, String>;
}

pub struct FileTextures {
    directory: PathBuf,
}
impl FileTextures {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }
}

fn case_insensitive_file(directory: &Path, name: &str) -> Option<PathBuf> {
    let direct = directory.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    std::fs::read_dir(directory)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

impl TextureSource for FileTextures {
    fn read(&self, name: &str) -> Result<TextureBytes, String> {
        // Max exporters frequently store machine-specific absolute Windows paths.
        let name = name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(name)
            .trim_matches('"');
        let path = case_insensitive_file(&self.directory.join("maps"), name)
            .or_else(|| case_insensitive_file(&self.directory, name))
            .ok_or_else(|| {
                format!(
                    "Texture {name} not found beside {}",
                    self.directory.display()
                )
            })?;
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        Ok(TextureBytes {
            key: path.to_string_lossy().into_owned(),
            extension: path
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("bmp")
                .to_ascii_lowercase(),
            bytes: std::fs::read(&path).map_err(|e| e.to_string())?,
        })
    }
}
