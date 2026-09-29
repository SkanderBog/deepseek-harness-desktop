/// 下载完成事件载荷：`on_download` 的 Finished 分支向前端 emit，
/// 由桌面外壳展示"已保存 + 打开文件夹"提示（iframe 内的下载对用户不可见）。
#[derive(Clone, serde::Serialize)]
pub struct DownloadFinishedPayload {
    /// 原始下载地址（dsh 的 /api/session.export?sessionId=...）
    pub(crate) url: String,
    /// 保存到本地的完整路径；失败或平台拿不到路径时为 None
    pub(crate) path: Option<String>,
    /// 下载是否成功
    pub(crate) success: bool,
}

/// 原生通知上的按钮：`action` 是与前端约定的动作 id，`title` 是按钮文案。
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeNotificationAction {
    pub(crate) action: String,
    pub(crate) title: String,
}

/// iframe 内 DSH 页面发来的原生通知请求载荷。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeNotificationPayload {
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) tag: Option<String>,
    /// 发出通知的会话 id；点击通知时原样回传，用于在壳层聚焦对应会话。
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    /// 需要渲染成系统通知按钮的动作；缺省（或空）表示纯展示型通知。
    #[serde(default)]
    pub(crate) actions: Option<Vec<NativeNotificationAction>>,
    /// 是否要求用户处理后才消失（授权/提问类通知为 true）。
    #[serde(default)]
    pub(crate) require_interaction: Option<bool>,
}
