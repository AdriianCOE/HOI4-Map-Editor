use std::env;
use std::path::PathBuf;

use super::Platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFontCandidate {
    pub path: PathBuf,
    pub index: u32,
}

impl SystemFontCandidate {
    fn new(path: impl Into<PathBuf>, index: u32) -> Self {
        Self {
            path: path.into(),
            index,
        }
    }
}

const WINDOWS_FONT_CANDIDATES: &[(&str, u32)] = &[
    ("segoeui.ttf", 0),
    ("seguisym.ttf", 0),
    ("msyh.ttc", 0),
    ("msyhbd.ttc", 0),
    ("simsun.ttc", 0),
    ("simhei.ttf", 0),
    ("tahoma.ttf", 0),
];

const LINUX_FONT_CANDIDATES: &[(&str, u32)] = &[
    // Noto CJK provides Simplified Chinese when the distribution includes it.
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
    ("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc", 0),
    // DejaVu and Noto Sans are common free Cyrillic-capable fallbacks.
    ("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", 0),
    ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0),
    (
        "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
        0,
    ),
    ("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc", 0),
];

pub fn system_font_candidates() -> Vec<SystemFontCandidate> {
    let system_root = env::var_os("SystemRoot").map(PathBuf::from);
    font_candidates(Platform::current(), system_root.as_deref())
}

pub fn font_candidates(
    platform: Platform,
    system_root: Option<&std::path::Path>,
) -> Vec<SystemFontCandidate> {
    match platform {
        Platform::Windows => {
            let fonts_dir = system_root
                .unwrap_or_else(|| std::path::Path::new("C:\\Windows"))
                .join("Fonts");
            WINDOWS_FONT_CANDIDATES
                .iter()
                .map(|(name, index)| SystemFontCandidate::new(fonts_dir.join(name), *index))
                .collect()
        }
        Platform::Linux => LINUX_FONT_CANDIDATES
            .iter()
            .map(|(path, index)| SystemFontCandidate::new(*path, *index))
            .collect(),
        Platform::MacOS | Platform::Other => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::font_candidates;
    use crate::platform::Platform;
    use std::path::Path;

    #[test]
    fn linux_candidates_include_cyrillic_and_cjk_fallbacks() {
        let candidates = font_candidates(Platform::Linux, None);
        let paths = candidates
            .iter()
            .map(|candidate| candidate.path.to_string_lossy())
            .collect::<Vec<_>>();

        assert!(paths.iter().any(|path| path.contains("DejaVuSans")));
        assert!(paths.iter().any(|path| path.contains("NotoSansCJK")));
    }

    #[test]
    fn windows_candidates_keep_the_system_root_and_order() {
        let candidates = font_candidates(Platform::Windows, Some(Path::new("C:/Windows")));
        assert_eq!(
            candidates[0].path,
            Path::new("C:/Windows/Fonts/segoeui.ttf")
        );
        assert_eq!(candidates[2].path, Path::new("C:/Windows/Fonts/msyh.ttc"));
    }
}
