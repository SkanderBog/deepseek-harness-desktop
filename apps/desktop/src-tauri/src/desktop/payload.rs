#[derive(Clone, serde::Serialize)]
pub struct DownloadFinishedPayload {
    /// 原始下载地址（dsh 的 /api/session.export?sessionId=...）
    pub(crate) url: String,
    /// 保存到本地的完整路径；失败或平台拿不到路径时为 None
    pub(crate) path: Option<String>,
    /// 下载是否成功
    pub(crate) success: bool,
}

