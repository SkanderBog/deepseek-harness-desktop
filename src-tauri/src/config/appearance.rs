use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "serde_json::Value")]
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

impl From<serde_json::Value> for Appearance {
    fn from(value: serde_json::Value) -> Self {
        let mut appearance = Self {
            palette: value["palette"].as_str().unwrap_or("default").to_owned(),
            terminal: value["terminal"].as_bool().unwrap_or(false),
            opacity: value["opacity"]
                .as_f64()
                .filter(|value| value.is_finite())
                .map(|value| value.round().clamp(20.0, 100.0) as u8)
                .unwrap_or(100),
        };
        appearance.normalize();
        appearance
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
    fn opacity_decoding_rounds_and_clamps_numeric_values() {
        for (value, expected) in [
            (serde_json::json!(77.49), 77),
            (serde_json::json!(77.5), 78),
            (serde_json::json!(20), 20),
            (serde_json::json!(100), 100),
            (serde_json::json!(-300), 20),
            (serde_json::json!(300), 100),
            (serde_json::json!(1e300), 100),
        ] {
            let appearance: Appearance = serde_json::from_value(serde_json::json!({
                "opacity": value
            }))
            .unwrap();
            assert_eq!(appearance.opacity, expected, "{value}");
        }
    }

    #[test]
    fn invalid_opacity_preserves_the_other_appearance_fields() {
        for value in [
            serde_json::json!(null),
            serde_json::json!("70"),
            serde_json::json!(true),
            serde_json::json!([]),
            serde_json::json!({}),
        ] {
            let appearance: Appearance = serde_json::from_value(serde_json::json!({
                "palette": "nord", "terminal": true, "opacity": value
            }))
            .unwrap();
            assert_eq!(
                appearance,
                Appearance {
                    palette: "nord".into(),
                    terminal: true,
                    opacity: 100
                },
                "{value}"
            );
        }
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
