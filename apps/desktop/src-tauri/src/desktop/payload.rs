#[derive(Clone, serde::Serialize)]
pub struct DownloadFinishedPayload {
    pub(crate) url: String,
    pub(crate) path: Option<String>,
    pub(crate) success: bool,
}

