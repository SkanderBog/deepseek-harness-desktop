use crate::WebviewWindow;
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{Emitter, Runtime};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressPayload {
    pub title: String,
    pub detail: String,
    pub log: String,
    pub r#type: String,
    pub percentage: f64,
    pub progress: f64,
}

pub struct ProgressTracker<'a, R: Runtime> {
    pub(super) window: &'a WebviewWindow<R>,
    total_phases: usize,
    current_phase: usize,
    current_title: String,
    current_type: String,
    last_emit_time: Mutex<Option<Instant>>,
}

impl<'a, R: Runtime> ProgressTracker<'a, R> {
    pub fn new(window: &'a WebviewWindow<R>, task_count: usize) -> Self {
        Self {
            window,
            total_phases: task_count,
            current_phase: 0,
            current_title: String::from("准备中..."),
            current_type: String::from(""),
            last_emit_time: Mutex::new(None),
        }
    }

    pub fn start_phase(&mut self, r#type: &str, title: &str) {
        self.current_title = title.to_string();
        self.current_type = r#type.to_string();
    }

    pub fn end_phase(&mut self) {
        if self.current_phase < self.total_phases {
            self.current_phase += 1;
        }
    }

    pub fn update(&self, stage_pct: f64, detail: String, log: String) {
        let now = Instant::now();
        let mut last_emit = self
            .last_emit_time
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        if let Some(last_time) = *last_emit {
            if now.duration_since(last_time) < Duration::from_millis(50) {
                return;
            }
        }

        *last_emit = Some(now);

        let global_pct = global_percentage(self.current_phase, self.total_phases, stage_pct);

        let _ = self.window.emit(
            "install-progress",
            ProgressPayload {
                title: self.current_title.clone(),
                r#type: self.current_type.clone(),
                percentage: global_pct,
                progress: stage_pct,
                detail,
                log,
            },
        );
    }

    pub fn skip_phases(&mut self, count: usize) {
        self.current_phase = (self.current_phase + count).min(self.total_phases);
    }
}

/// = 116.7 ≈ 117%`，正是 issue #283 首页「第一次安装卡住」误显示 117% 的根因）。
fn global_percentage(current_phase: usize, total_phases: usize, stage_pct: f64) -> f64 {
    if total_phases == 0 {
        return 100.0;
    }
    let phase_weight = 100.0 / total_phases as f64;
    let global = (current_phase as f64 * phase_weight) + (stage_pct * phase_weight / 100.0);
    global.clamp(0.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::global_percentage;

    #[test]
    fn clamps_to_100_when_all_phases_done_plus_final_update() {
        // issue #283：macOS 首装 3 任务 × 2 阶段 = 6 阶段，全部结束后再补一次
        let pct = global_percentage(6, 6, 100.0);
        assert_eq!(pct, 100.0);
    }

    #[test]
    fn stays_below_100_mid_phase() {
        let pct = global_percentage(0, 6, 50.0);
        assert!((pct - 8.3333).abs() < 1e-3);
        assert!(pct < 100.0);
    }

    #[test]
    fn reaches_100_exactly_at_last_phase_end() {
        let pct = global_percentage(5, 6, 100.0);
        assert_eq!(pct, 100.0);
    }

    #[test]
    fn total_phases_one_and_mid() {
        let done = global_percentage(1, 1, 100.0);
        assert_eq!(done, 100.0);
        let half = global_percentage(0, 1, 50.0);
        assert_eq!(half, 50.0);
    }

    #[test]
    fn never_exceeds_100_regardless_of_input() {
        for current in 0..=10 {
            for stage in [0.0, 25.0, 50.0, 75.0, 100.0, 200.0] {
                let pct = global_percentage(current, 6, stage);
                assert!(pct <= 100.0, "pct={pct} current={current} stage={stage}");
            }
        }
    }

    #[test]
    fn degenerate_zero_total_is_safe() {
        assert_eq!(global_percentage(0, 0, 100.0), 100.0);
    }
}
