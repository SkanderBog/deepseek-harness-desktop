use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub palette: String,
    pub terminal: bool,
    pub opacity: u8,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            palette: "default".into(),
            terminal: false,
            opacity: 100,
        }
    }
}

impl Appearance {
    pub fn normalize(&mut self) {
        if !matches!(
            self.palette.as_str(),
            "default" | "nord" | "solarized" | "forest" | "amber"
        ) {
            self.palette = "default".into();
        }
        self.opacity = self.opacity.clamp(20, 100);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_appearance_fields_preserve_the_original_window() {
        let appearance: Appearance = serde_json::from_str("{}").unwrap();
        assert_eq!(appearance, Appearance::default());
        assert_eq!(appearance.opacity, 100);
        assert!(!appearance.terminal);
    }

    #[test]
    fn invalid_palette_and_opacity_are_normalized() {
        let mut appearance = Appearance {
            palette: "unknown".into(),
            terminal: true,
            opacity: 0,
        };
        appearance.normalize();
        assert_eq!(appearance.palette, "default");
        assert_eq!(appearance.opacity, 20);
        assert!(appearance.terminal);
        appearance.opacity = 255;
        appearance.normalize();
        assert_eq!(appearance.opacity, 100);
    }
}
