pub(crate) const NOTIFICATION_SHIM_JS: &str = r#"(function () {
  if (window.__dsh_native_notification_bridge__) return;
  window.__dsh_native_notification_bridge__ = true;

  var hostHidden = false;
  var pendingOnClicks = {};

  function setHostHidden(hidden) {
    hidden = !!hidden;
    if (hostHidden === hidden) return;
    hostHidden = hidden;
    try { document.dispatchEvent(new Event('visibilitychange')); } catch (_) {}
  }

  function send(message) {
    try {
      window.parent.postMessage(Object.assign({ source: 'dsh-notification-bridge' }, message), '*');
    } catch (_) {}
  }

  function sessionIdFromTag(tag) {
    var m = /^dsh-notification-(?:pending-)?(.+)-\d+$/.exec(tag || '');
    return m ? m[1] : '';
  }

  function DshNativeNotification(title, options) {
    options = options || {};
    this.title = String(title || '');
    this.options = options;
    this.tag = options.tag || '';
    this.onclick = null;
    this.onaction = null;
    this.onclose = null;
    this.onerror = null;
    this.onshow = null;

    if (this.tag) pendingOnClicks[this.tag] = this;

    send({
      type: 'dsh://native-notification',
      title: this.title,
      body: String(options.body || ''),
      tag: this.tag,
      requireInteraction: !!options.requireInteraction,
      silent: !!options.silent,
      sessionId: options.sessionId || sessionIdFromTag(this.tag),
      actions: Array.isArray(options.actions) ? options.actions : [],
      href: location.href,
      origin: location.origin
    });
  }

  DshNativeNotification.permission = 'granted';
  DshNativeNotification.requestPermission = function () {
    return Promise.resolve('granted');
  };
  DshNativeNotification.prototype.close = function () {
    if (!this.tag) return;
    // 关掉后不会再回灌点击：及时释放实例，避免 pendingOnClicks 无限增长。
    delete pendingOnClicks[this.tag];
    send({ type: 'dsh://close-notification', tag: this.tag });
  };

  window.Notification = DshNativeNotification;

  // 属性拦截：让 iframe 中的 visibilityState 强行跟随桌面宿主状态
  (function installHiddenOverrides() {
    var descMap = {
      hidden: { configurable: true, get: function () { return hostHidden; } },
      visibilityState: { configurable: true, get: function () { return hostHidden ? 'hidden' : 'visible'; } }
    };
    [document, Object.getPrototypeOf(document)].forEach(function (target) {
      if (!target) return;
      try {
        Object.defineProperty(target, 'hidden', descMap.hidden);
        Object.defineProperty(target, 'visibilityState', descMap.visibilityState);
      } catch (_) {}
    });
  })();

  // 宿主状态只认壳层推送（`dsh://visibility-state`）与「帧内拿到焦点」这一条正向证据。
  // 以前这里还有 `blur → 隐藏`，但帧失焦 ≠ 窗口隐藏：点壳层标题栏、设置面板都会让
  // 帧 blur，而窗口仍在前台；那条误判会让「仅在未聚焦时」在用户正盯着会话时弹通知。
  // 帧拿到焦点则窗口必然在前台（最小化的窗口给不了焦点），所以 focus 可以安全地纠正。
  window.addEventListener('focus', function () { setHostHidden(false); });

  // 查找并聚焦对应的 Session
  function tryFocusSession(sessionId, title) {
    if (!sessionId && !title) return;

    if (typeof window.__dsh_focusSession === 'function') {
      try { window.__dsh_focusSession(sessionId); return; } catch (_) {}
    }

    if (sessionId) {
      var el = document.querySelector('[data-session-id="' + sessionId + '"], [data-session="' + sessionId + '"], [data-id="' + sessionId + '"]');
      if (el) { el.click(); return; }
    }

    if (title) {
      var trimmedTitle = String(title).trim();
      var nodes = document.querySelectorAll('button, [role="button"], li, a');
      for (var i = 0; i < nodes.length; i++) {
        if (nodes[i].offsetParent !== null && nodes[i].textContent.trim() === trimmedTitle) {
          nodes[i].click();
          return;
        }
      }
    }
  }

  window.addEventListener('message', function (event) {
    var data = event.data;
    if (!data || typeof data !== 'object') return;

    switch (data.type) {
      case 'dsh://visibility-state':
        setHostHidden(data.hidden);
        break;
      case 'dsh://focus-session':
        tryFocusSession(data.sessionId, data.title);
        break;
      case 'dsh://notification-clicked':
        var instance = pendingOnClicks[data.tag];
        if (!instance) break;
        // 一次点击只回灌一次；用完即弃，避免长时间运行后字典持续增长。
        delete pendingOnClicks[data.tag];
        // 按钮点击走 onaction（动作 id 由前端约定），点通知本体走 onclick。
        // 带输入框的按钮（如「回复」）把用户填的文本一并回灌；没填时给空串。
        if (data.action && typeof instance.onaction === 'function') {
          var inputValue = typeof data.inputValue === 'string' ? data.inputValue : '';
          try { instance.onaction({ action: String(data.action), tag: data.tag, inputValue: inputValue }); } catch (_) {}
        } else if (typeof instance.onclick === 'function') {
          try { instance.onclick(new Event('click')); } catch (_) {}
        }
        break;
    }
  });
})();"#;
