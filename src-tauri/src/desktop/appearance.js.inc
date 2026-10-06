(function () {
  if (window.__dsh_appearance_bootstrap__) return
  window.__dsh_appearance_bootstrap__ = true
  // 只有宿主直接内嵌的那一层 frame 参与握手：dsh 应用内部还会嵌套 iframe，
  // 它们的父窗口是 dsh 页面而不是宿主，自报/应答都不该走这套协议。
  // （直达宿主的帧里 parent === top；嵌套帧的 parent 是 dsh 页面，不等于 top。）
  if (window.parent !== window.top) return

  // 宿主与内嵌 dsh 服务**跨源**（壳层是 tauri://localhost / http://tauri.localhost，
  // 帧内是 http://127.0.0.1:<port>，见 tauri.conf.json 的 frame-src），所以回发一律
  // 用 `'*'`：用帧内 location.origin 当 targetOrigin 会让消息静默丢失（这正是外观插件
  // `invokeParent` 的既有做法）。消息可信度由宿主的 source + origin 校验、以及帧内的
  // `event.source === window.parent` + `data.source` 共同保证，请求本身不含敏感数据。
  var STYLE_ID = 'dsh-tauri:boot-appearance'
  var maxRequests = 12
  var requestDelay = 40
  var requestTimer = null
  var requests = 0
  var acknowledged = false

  // head/documentElement 在 document-start 可能还不存在，等它们出现再补挂。
  function mountTarget() {
    return document.head || document.documentElement
  }

  function bootStyle() {
    var style = document.getElementById(STYLE_ID)
    if (style) return style
    style = document.createElement('style')
    style.id = STYLE_ID
    var target = mountTarget()
    if (!target) return null
    target.appendChild(style)
    return style
  }

  // boot 页由内核在插件加载之前绘出，此时外观插件尚未激活；宿主下发的 CSS 是它唯一
  // 的透明来源。样式始终挂在独立 head style 上，绝不改 boot 节点的 DOM（BootHandoff
  // 只交接 className/innerHTML，额外 inline style 会被判成 hydration 不匹配）。
  function apply(css) {
    var style = bootStyle()
    if (!style) return false
    if (style.textContent !== css) style.textContent = css
    return true
  }

  function acknowledge() {
    if (acknowledged) return
    acknowledged = true
    if (requestTimer !== null) {
      clearTimeout(requestTimer)
      requestTimer = null
    }
    parent.postMessage({ source: 'dsh-desktop', type: 'dsh://appearance:applied' }, '*')
  }

  // 请求可能早于壳层的监听器就位（iframe 与 style 同一次提交挂载），用有界退避重试；
  // 上限用尽后停在未确认态，由宿主侧超时兜底揭开 iframe，不会永久挡住建。
  function requestAppearance() {
    if (acknowledged || requests >= maxRequests) return
    if (requestTimer !== null) clearTimeout(requestTimer)
    requests++
    parent.postMessage({ source: 'dsh-desktop', type: 'dsh://appearance:request' }, '*')
    requestTimer = setTimeout(requestAppearance, requestDelay)
    requestDelay = Math.min(requestDelay * 2, 400)
  }

  window.addEventListener('message', function (event) {
    if (event.source !== window.parent) return
    var data = event.data
    if (!data || typeof data !== 'object') return
    if (data.source !== 'dsh-desktop') return
    if (data.type !== 'dsh://appearance') return
    if (!apply(typeof data.bootCss === 'string' ? data.bootCss : '')) {
      // 文档骨架还没建好：立刻重试，别把这次下发丢掉。
      requestAppearance()
      return
    }
    acknowledge()
  })

  // 帧内导航离开：旧文档的 boot 样式立即失效，别让它短暂作用在新文档上；新文档自己
  // 重新握手（同文档 bfcache 还原走下面的 pageshow，宿主据 leaving 重新隐蔽 iframe）。
  window.addEventListener('pagehide', function () {
    var style = document.getElementById(STYLE_ID)
    if (style) style.remove()
    acknowledged = false
    requests = 0
    requestDelay = 40
    if (requestTimer !== null) {
      clearTimeout(requestTimer)
      requestTimer = null
    }
  })

  // bfcache 还原 / 文档被再次激活时重新握手：宿主可能已经换过外观，boot 页也可能
  // 重新挂载（页面重载后 style 标签随旧文档一起消失）。
  window.addEventListener('pageshow', function () {
    acknowledged = false
    requests = 0
    requestDelay = 40
    requestAppearance()
  })

  requestAppearance()
})()
