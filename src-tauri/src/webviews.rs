//! WebView 创建、布局与注入脚本。
//!
//! 账号起始 URL 的计算也在这里（`normalized_global_default_url` /
//! `effective_profile_home_url` / `profile_start_url`）。这两条是 URL 规则的
//! **唯一真值**，前端 `src/lib/profileUrl.ts` 只做即时反馈，改规则必须改这里。

use chrono::Utc;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{
    webview::{NewWindowResponse, WebviewBuilder, WebviewWindowBuilder},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Runtime, WebviewUrl,
};
use url::Url;

use crate::*;

pub(crate) fn profile_label(id: &str) -> String {
    format!("{PROFILE_PREFIX}{}", id.replace('-', "_"))
}

pub(crate) fn profile_data_dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(base.join("profiles").join(id))
}

/// 从账号 UUID 派生 32 位种子（FNV-1a），作为该账号指纹噪声的确定性来源：
/// 同一账号每次读到同一噪声，不同账号对同一页面读到不同噪声。
pub(crate) fn profile_seed(id: &str) -> u32 {
    let mut seed: u32 = 0x811c_9dc5;
    for byte in id.as_bytes() {
        seed ^= u32::from(*byte);
        seed = seed.wrapping_mul(0x0100_0193);
    }
    seed
}

/// 修复旧版/异常账号中的空网址或坏网址。
/// 读取即修复并落盘，确保之后休眠恢复、重启恢复、直接打开都不会再次卡住。
pub(crate) fn normalized_global_default_url<R: Runtime>(app: &AppHandle<R>) -> Url {
    let settings = load_app_settings(app);
    normalize_url(&settings.global_default_url).unwrap_or_else(|_| {
        Url::parse(DEFAULT_PROFILE_URL).expect("DEFAULT_PROFILE_URL must be valid")
    })
}

pub(crate) fn effective_profile_home_url<R: Runtime>(app: &AppHandle<R>, profile: &Profile) -> Url {
    if profile.url_mode == "inherit" {
        return normalized_global_default_url(app);
    }
    normalize_url(&profile.default_url).unwrap_or_else(|_| normalized_global_default_url(app))
}

pub(crate) fn repair_profile_urls<R: Runtime>(app: &AppHandle<R>, profile: &mut Profile) -> bool {
    let mut changed = false;
    if profile.url_mode != "inherit" && profile.url_mode != "custom" {
        profile.url_mode = "custom".to_string();
        changed = true;
    }
    let fallback = normalized_global_default_url(app).to_string();
    let default_url = normalize_url(&profile.default_url)
        .map(|u| u.to_string())
        .unwrap_or_else(|_| fallback.clone());
    if profile.default_url != default_url {
        profile.default_url = default_url;
        changed = true;
    }
    let home = effective_profile_home_url(app, profile).to_string();
    let last_url = if profile.last_url.trim().is_empty() {
        home.clone()
    } else {
        normalize_url(&profile.last_url)
            .map(|u| u.to_string())
            .unwrap_or(home)
    };
    if profile.last_url != last_url {
        profile.last_url = last_url;
        changed = true;
    }
    changed
}

pub(crate) fn profile_start_url(app: &AppHandle, profile: &Profile) -> Url {
    if !profile.last_url.trim().is_empty() {
        if let Ok(url) = normalize_url(&profile.last_url) {
            return url;
        }
    }
    effective_profile_home_url(app, profile)
}

pub(crate) fn update_last_url(app: &AppHandle, id: &str, url: &str) {
    if let Ok(mut profiles) = load_profiles(app) {
        if let Some(profile) = profiles.iter_mut().find(|p| p.id == id) {
            // SPA 的同地址导航（hash 变化、弹层路由等）不用反复写盘。
            if profile.last_url == url {
                return;
            }
            profile.last_url = url.to_string();
            let _ = save_profiles(app, &profiles);
        }
    }
}

/// 统一布局所有账号 WebView：全部对齐到当前视口矩形，显示活跃的、隐藏其余。
/// 隐藏的也同步边界，避免切换标签时旧 WebView 因边界残留从边缘“漏”出来。
pub(crate) fn layout_profile_webviews(app: &AppHandle, active_label: &str, bounds: &BrowserBounds) {
    let position = LogicalPosition::new(bounds.x, bounds.y);
    let size = LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0));
    for (label, webview) in app.webviews() {
        if !label.starts_with(PROFILE_PREFIX) {
            continue;
        }
        let active = label == active_label;
        if active {
            // 只给当前可见 WebView 同步几何尺寸。后台几十个 WebView 若每次切换/缩放
            // 都 set_position + set_size，会产生大量原生 IPC 并明显拖慢窗口。
            let _ = webview.set_position(position);
            let _ = webview.set_size(size);
            let _ = webview.show();
        } else {
            let _ = webview.hide();
        }
        // 原生子 WebView 的 show/hide 不同平台对 document.hidden 的反馈并不一致，
        // 显式告诉注入脚本谁在前台，后台账号就停止高频状态扫描。
        let _ = webview.eval(profile_activity_eval_script(active));
    }
}

/// 画布 / WebGL / 硬件指纹防护。噪声由账号种子确定性生成，保证
/// 同账号跨页面、跨会话一致，不同账号之间互不相同。
const FINGERPRINT_GUARD_JS: &str = r#"
(function () {
  if (window.__mbGuardInstalled__) return;
  try { Object.defineProperty(window, '__mbGuardInstalled__', { value: true }); } catch (e) { return; }
  var seed = __SEED__ >>> 0;

  function nativeFn(fn, name) {
    try {
      Object.defineProperty(fn, 'toString', { value: function () { return 'function ' + name + '() { [native code] }'; }, writable: true, configurable: true });
      Object.defineProperty(fn, 'name', { value: name, writable: true, configurable: true });
    } catch (e) {}
    return fn;
  }

  function noiseAt(r, g, b, x, y) {
    var h = seed ^ Math.imul(x + 1, 374761393) ^ Math.imul(y + 1, 668265263) ^ Math.imul((r << 16) | (g << 8) | b, 2654435761);
    h = Math.imul(h ^ (h >>> 13), 1274126177);
    h ^= h >>> 16;
    return ((h >>> 0) % 3) - 1;
  }

  function clamp(v) { return v < 0 ? 0 : v > 255 ? 255 : v; }

  function perturbData(imageData) {
    try {
      var data = imageData.data, w = imageData.width | 0, h = imageData.height | 0;
      var total = w * h;
      if (!total) return;
      var step = total > 262144 ? 8 : 1;
      for (var p = 0; p < total; p += step) {
        var i = p << 2;
        var nz = noiseAt(data[i], data[i + 1], data[i + 2], p % w, (p / w) | 0);
        if (nz) {
          data[i] = clamp(data[i] + nz);
          data[i + 1] = clamp(data[i + 1] - nz);
        }
      }
    } catch (e) {}
  }

  function patchImageData(proto) {
    if (!proto || !proto.getImageData) return;
    var original = proto.getImageData;
    proto.getImageData = nativeFn(function () {
      var imageData = original.apply(this, arguments);
      perturbData(imageData);
      return imageData;
    }, 'getImageData');
  }

  try { patchImageData(CanvasRenderingContext2D.prototype); } catch (e) {}
  try { if (window.OffscreenCanvasRenderingContext2D) patchImageData(OffscreenCanvasRenderingContext2D.prototype); } catch (e) {}

  // toDataURL / toBlob：把画布复制到临时画布再读取，噪声只作用于读出结果，不改用户画布。
  function readNoisedCopy(canvas) {
    var tmp = document.createElement('canvas');
    tmp.width = canvas.width;
    tmp.height = canvas.height;
    var tctx = tmp.getContext('2d');
    if (!tctx) return null;
    tctx.drawImage(canvas, 0, 0);
    var data = tctx.getImageData(0, 0, tmp.width, tmp.height);
    tctx.putImageData(data, 0, 0);
    return tmp;
  }

  try {
    var originalToDataURL = HTMLCanvasElement.prototype.toDataURL;
    HTMLCanvasElement.prototype.toDataURL = nativeFn(function () {
      try {
        if (this.width && this.height) {
          var copy = readNoisedCopy(this);
          if (copy) return originalToDataURL.apply(copy, arguments);
        }
      } catch (e) {}
      return originalToDataURL.apply(this, arguments);
    }, 'toDataURL');

    var originalToBlob = HTMLCanvasElement.prototype.toBlob;
    HTMLCanvasElement.prototype.toBlob = nativeFn(function (callback) {
      var rest = Array.prototype.slice.call(arguments, 1);
      var target = this;
      try {
        if (this.width && this.height) {
          var copy = readNoisedCopy(this);
          if (copy) {
            target = copy;
          }
        }
      } catch (e) {}
      return originalToBlob.apply(target, [callback].concat(rest));
    }, 'toBlob');
  } catch (e) {}

  // WebGL GPU 字符串掩码（UNMASKED_VENDOR_WEBGL / UNMASKED_RENDERER_WEBGL）。
  var glPick = seed % 3;
  var glVendors = ['Google Inc. (Intel)', 'Google Inc. (NVIDIA)', 'Google Inc. (AMD)'];
  var glRenderers = [
    'ANGLE (Intel, Intel(R) UHD Graphics 630 (0x00003E9B) Direct3D11 vs_5_0 ps_5_0, D3D11)',
    'ANGLE (NVIDIA, NVIDIA GeForce GTX 1650 (0x00001F82) Direct3D11 vs_5_0 ps_5_0, D3D11)',
    'ANGLE (AMD, AMD Radeon(TM) Graphics (0x00001638) Direct3D11 vs_5_0 ps_5_0, D3D11)'
  ];
  function patchGetParameter(proto) {
    if (!proto || !proto.getParameter) return;
    var original = proto.getParameter;
    proto.getParameter = nativeFn(function (param) {
      if (param === 37445) return glVendors[glPick];
      if (param === 37446) return glRenderers[glPick];
      return original.apply(this, arguments);
    }, 'getParameter');
  }
  try { if (window.WebGLRenderingContext) patchGetParameter(WebGLRenderingContext.prototype); } catch (e) {}
  try { if (window.WebGL2RenderingContext) patchGetParameter(WebGL2RenderingContext.prototype); } catch (e) {}

  // 音频指纹（AudioContext/OfflineAudioContext 渲染摘要）：按账号种子给
  // AudioBuffer 采样加确定性微噪声（±2e-7，听感无差异，摘要完全改变）。
  // getChannelData 对同一 buffer 返回缓存数组，打标记防止重复叠加漂移。
  function perturbSamples(data) {
    try {
      if (!data || !data.length || data.length > 1000000) return;
      if (data.__mbHNoised) return;
      for (var i = 0; i < data.length; i++) {
        var h = Math.imul(i + 1, 2654435761) ^ seed;
        h = Math.imul(h ^ (h >>> 13), 1274126177);
        h ^= h >>> 16;
        data[i] += (((h >>> 0) % 5) - 2) * 1e-7;
      }
      Object.defineProperty(data, '__mbHNoised', { value: true });
    } catch (e) {}
  }

  try {
    var bufferProto = AudioBuffer.prototype;
    var originalGetChannelData = bufferProto.getChannelData;
    bufferProto.getChannelData = nativeFn(function () {
      var data = originalGetChannelData.apply(this, arguments);
      perturbSamples(data);
      return data;
    }, 'getChannelData');
    if (bufferProto.copyFromChannel) {
      var originalCopyFromChannel = bufferProto.copyFromChannel;
      bufferProto.copyFromChannel = nativeFn(function (destination) {
        originalCopyFromChannel.apply(this, arguments);
        perturbSamples(destination);
      }, 'copyFromChannel');
    }
  } catch (e) {}

  try {
    var analyserProto = AnalyserNode.prototype;
    ['getFloatFrequencyData', 'getFloatTimeDomainData'].forEach(function (name) {
      var original = analyserProto[name];
      analyserProto[name] = nativeFn(function (array) {
        var result = original.apply(this, arguments);
        perturbSamples(array);
        return result;
      }, name);
    });
  } catch (e) {}

  // 硬件信息按账号固定。
  var cores = [4, 6, 8, 12, 16][seed % 5];
  var mem = [4, 8, 8, 16, 8][seed % 5];
  try { Object.defineProperty(navigator, 'hardwareConcurrency', { get: function () { return cores; }, configurable: true }); } catch (e) {}
  try { Object.defineProperty(navigator, 'deviceMemory', { get: function () { return mem; }, configurable: true }); } catch (e) {}
})();
"#;

/// 时区伪装：接管 getTimezoneOffset 与默认 Intl.DateTimeFormat。
const TIMEZONE_JS: &str = r#"
(function () {
  var TZ = '__TZ__';
  try { new Intl.DateTimeFormat('en', { timeZone: TZ }); } catch (e) { return; }

  function nativeFn(fn, name) {
    try {
      Object.defineProperty(fn, 'toString', { value: function () { return 'function ' + name + '() { [native code] }'; }, writable: true, configurable: true });
      Object.defineProperty(fn, 'name', { value: name, writable: true, configurable: true });
    } catch (e) {}
    return fn;
  }

  function zoneOffsetMinutes(date) {
    var dtf = new Intl.DateTimeFormat('en-US', {
      timeZone: TZ, hour12: false,
      year: 'numeric', month: '2-digit', day: '2-digit',
      hour: '2-digit', minute: '2-digit', second: '2-digit'
    });
    var parts = dtf.formatToParts(date), map = {};
    for (var i = 0; i < parts.length; i++) map[parts[i].type] = parts[i].value;
    var asUTC = Date.UTC(+map.year, map.month - 1, +map.day, (+map.hour) % 24, +map.minute, +map.second);
    return Math.round((asUTC - date.getTime()) / 60000);
  }

  Date.prototype.getTimezoneOffset = nativeFn(function () {
    return -zoneOffsetMinutes(this);
  }, 'getTimezoneOffset');

  var RealDateTimeFormat = Intl.DateTimeFormat;
  function PatchedDateTimeFormat() {
    var locales = arguments[0];
    var options = arguments[1];
    if (!options || options.timeZone === undefined) {
      var patched = Object.assign({}, options || {}, { timeZone: TZ });
      return new RealDateTimeFormat(locales, patched);
    }
    return new RealDateTimeFormat(locales, options);
  }
  PatchedDateTimeFormat.prototype = RealDateTimeFormat.prototype;
  try { Object.defineProperty(PatchedDateTimeFormat, 'supportedLocalesOf', { value: RealDateTimeFormat.supportedLocalesOf }); } catch (e) {}
  Intl.DateTimeFormat = PatchedDateTimeFormat;
})();
"#;

/// 语言伪装：接管 navigator.language(s) 与 Intl 各格式化类的默认区域。
const LOCALE_JS: &str = r#"
(function () {
  var L = '__LOCALE__';

  function nativeFn(fn, name) {
    try {
      Object.defineProperty(fn, 'toString', { value: function () { return 'function ' + name + '() { [native code] }'; }, writable: true, configurable: true });
      Object.defineProperty(fn, 'name', { value: name, writable: true, configurable: true });
    } catch (e) {}
    return fn;
  }

  try {
    Object.defineProperty(navigator, 'language', { get: function () { return L; }, configurable: true });
    Object.defineProperty(navigator, 'languages', { get: function () { return [L]; }, configurable: true });
  } catch (e) {}

  ['DateTimeFormat', 'NumberFormat', 'Collator', 'PluralRules', 'RelativeTimeFormat', 'ListFormat', 'Segmenter', 'DisplayNames']
    .forEach(function (name) {
      try {
        var Real = Intl[name];
        if (typeof Real !== 'function') return;
        var Patched = function () {
          var args = Array.prototype.slice.call(arguments);
          if (args[0] === undefined || args[0] === null || args[0] === '') args[0] = L;
          return Reflect.construct(Real, args);
        };
        Patched.prototype = Real.prototype;
        if (Real.supportedLocalesOf) {
          Object.defineProperty(Patched, 'supportedLocalesOf', { value: Real.supportedLocalesOf });
        }
        Intl[name] = Patched;
      } catch (e) {}
    });
})();
"#;

/// 自定义 UA 去掉 Edge 标识时，同步覆盖 navigator.userAgentData，
/// 避免“UA 字符串说 Chrome、Client Hints 说 Edge”的矛盾特征。
/// 注意：Sec-CH-UA 请求头由网络层生成，JS 无法改，属 WebView2 硬限制。
const USER_AGENT_DATA_JS: &str = r#"
(function () {
  if (!navigator.userAgentData) return;
  var brands = __BRANDS__;
  var fullVersion = '__FULL__';
  var mobile = __MOBILE__;
  var platform = '__PLATFORM__';
  var original = navigator.userAgentData;
  var override = {
    brands: brands,
    mobile: mobile,
    platform: platform
  };
  override.getHighEntropyValues = function (hints) {
    return original.getHighEntropyValues(hints).then(function (values) {
      try {
        hints = hints || [];
        if (hints.indexOf('fullVersionList') !== -1) values.fullVersionList = brands;
        if (hints.indexOf('uaFullVersion') !== -1) values.uaFullVersion = fullVersion;
        if (hints.indexOf('platform') !== -1) values.platform = platform;
        if (hints.indexOf('model') !== -1) values.model = '';
      } catch (e) {}
      return values;
    });
  };
  override.toJSON = function () {
    return { brands: brands, mobile: mobile, platform: platform };
  };
  try { Object.defineProperty(navigator, 'userAgentData', { get: function () { return override; }, configurable: true }); } catch (e) {}
})();
"#;

/// 新窗口 / blob 下载链接 / 图片预览补丁：宿主会把 target=_blank 与 window.open
/// 请求重定向回当前页（见 on_new_window），但 blob:/data: 这类内存下载链接不能
/// 整页导航（blob URL 无法跨上下文加载），这里在页面内提前接管：
/// 摘掉 target 让锚点留在本页触发 WebView2 下载管理器，window.open(blob:)
/// 则合成一个带 download 属性的锚点点击完成下载。
///
/// 图片另走一路：站点里的图片普遍是 `<a target="_blank" href=".../x.png">`，
/// 按上面那条规则摘掉 target 后，整个账号页会被图片顶掉（用户丢掉当前会话）。
/// 这里识别出图片链接后不改导航，而是回传 mbimage:// 假导航，由宿主另开小窗。
pub(crate) const NEW_WINDOW_PATCH_JS: &str = r#"
(function () {
  if (window.__mbNewTabInstalled) return;
  try { Object.defineProperty(window, '__mbNewTabInstalled', { value: true }); } catch (e) { return; }

  function isMemoryUrl(u) { return /^(blob:|data:)/i.test(String(u || '')); }
  function isDownloadLike(el, href) {
    try {
      if (el && el.hasAttribute && el.hasAttribute('download')) return true;
      var raw = String(href || '');
      if (isMemoryUrl(raw)) return true;
      var clean = raw.split('#')[0].split('?')[0];
      if (/\.(?:md|txt|csv|docx?|xlsx?|pptx?|pdf|png|jpe?g|gif|webp|zip|rar|7z|json|html?|xml|mp3|wav|mp4|mov|webm)$/i.test(clean)) return true;
      if (/(?:^|[\/?&=_-])(?:download|export|attachment|file)(?:[\/?&=_-]|$)/i.test(raw)) return true;
      var text = el ? String(el.textContent || el.getAttribute('aria-label') || el.getAttribute('title') || '') : '';
      return /(?:下载|导出|保存文件|download|export|save file)/i.test(text);
    } catch (e) { return false; }
  }

  // 图片地址识别：blob: 一定是（上传后的本地预览），其余看扩展名。
  // 不用「链接里包着 img 就算图片」这种宽口径 —— 卡片列表的缩略图也是这个结构，
  // 宽口径会把「点缩略图进详情页」也变成弹小窗，那是误伤。
  var IMAGE_EXT_RE = /\.(?:png|jpe?g|gif|webp|bmp|svg|avif|ico|heic|heif|jfif|tiff?)$/i;
  function isImageUrl(u) {
    try {
      var raw = String(u == null ? '' : u).trim();
      if (!raw) return false;
      if (/^blob:/i.test(raw)) return true;
      if (!/^(?:https?:)?\/\//i.test(raw)) return false;
      return IMAGE_EXT_RE.test(raw.split('#')[0].split('?')[0]);
    } catch (e) { return false; }
  }

  function absoluteUrl(url) {
    try { return new URL(String(url), location.href).href; } catch (e) { return String(url || ''); }
  }

  // 请求宿主另开小窗显示图片。宿主在 on_navigation 里取消这次假导航，页面本身不跳转。
  function previewImage(url, label) {
    try {
      var u = absoluteUrl(url);
      if (!u) return false;
      location.href = 'mbimage://open?u=' + encodeURIComponent(u)
        + '&t=' + encodeURIComponent(String(label || '').slice(0, 120));
      return true;
    } catch (e) { return false; }
  }

  function nearestImage(node) {
    try {
      var n = node;
      while (n && n.nodeType === 1) {
        if (n.tagName === 'IMG') return n;
        if (n.tagName === 'A') return n.querySelector ? n.querySelector('img') : null;
        n = n.parentElement;
      }
      return null;
    } catch (e) { return null; }
  }

  function placeholderHref(href) {
    var h = String(href || '').trim().toLowerCase();
    return h === '' || h === '#' || h.indexOf('javascript:') === 0;
  }

  function imgLabel(img) {
    try { return String(img.getAttribute('alt') || img.getAttribute('title') || ''); } catch (e) { return ''; }
  }

  // WebView2 的新窗口请求会被宿主拒绝。对于用户点击的链接，直接摘掉
  // target=_blank，让请求留在当前账号 WebView 中；这样服务端返回
  // Content-Disposition: attachment 时才能进入 on_download，而不是先变成 popup。
  // 图片链接例外：改成开预览小窗，账号页保持原样。
  document.addEventListener('click', function (e) {
    try {
      if (e.defaultPrevented || e.button !== 0) return;
      // 只接管真人点击。站点和宿主自己的自动下载都靠 element.click() 触发，
      // 那种合成事件的 isTrusted 是 false —— 如果也拦成预览小窗，
      // 「AI 文件自动下载」遇到图片白名单就会被整条打断。
      if (!e.isTrusted) return;
      var el = e.target;
      while (el && el.tagName !== 'A') el = el.parentElement;
      if (!el) return;
      if (!el.hasAttribute('download')) {
        var img = nearestImage(e.target);
        var href = el.getAttribute('href') || '';
        // 先补成绝对地址再判断：站点里相对路径的图片链接（/files/x.png）同样常见。
        var absolute = href ? absoluteUrl(href) : '';
        if (isImageUrl(absolute)) {
          var label = String(el.getAttribute('title') || el.getAttribute('aria-label') || '') || imgLabel(img);
          if (previewImage(absolute, label)) { e.preventDefault(); e.stopPropagation(); return; }
        } else if (img && placeholderHref(href)) {
          // 链接没有真地址、只包着图片（站点自己用 JS 处理点击）：图片本身就是目标。
          var src = img.currentSrc || img.src || '';
          if (isImageUrl(src) && previewImage(src, imgLabel(img))) { e.preventDefault(); e.stopPropagation(); return; }
        }
      }
      var target = (el.getAttribute('target') || '').toLowerCase();
      if (target === '_blank') el.removeAttribute('target');
    } catch (x) {}
  }, true);

  // 站点经常通过 a.click() 触发下载，这种点击不会经过用户侧的 target 修补时机。
  // 在原型层也做同样兜底，尤其覆盖 blob:/data: 和带 download 属性的动态链接。
  try {
    var originalAnchorClick = HTMLAnchorElement.prototype.click;
    HTMLAnchorElement.prototype.click = function () {
      try {
        if ((this.getAttribute('target') || '').toLowerCase() === '_blank' || isDownloadLike(this, this.href)) {
          this.removeAttribute('target');
        }
      } catch (e) {}
      return originalAnchorClick.apply(this, arguments);
    };
  } catch (x) {}

  try {
    var originalOpen = window.open;
    window.open = function (url) {
      try {
        var urlStr = String(url == null ? '' : url);
        // 站点自己 window.open 图片时，原来会走下面的「下载类地址」分支
        // （.png 命中扩展名）整页导航，账号页同样被顶掉 —— 先拦成预览小窗。
        var absolute = urlStr ? absoluteUrl(urlStr) : '';
        if (isImageUrl(absolute) && previewImage(absolute, '')) return null;
        if (isMemoryUrl(url)) {
          var a = document.createElement('a');
          a.href = url;
          a.download = '';
          document.documentElement.appendChild(a);
          a.click();
          a.remove();
          return null;
        }
        // 明显的下载 URL 不创建 popup，改为当前 WebView 导航，从而交给
        // WebView2 下载管理器处理 Content-Disposition 响应。
        if (url && isDownloadLike(null, url)) {
          location.href = String(url);
          return null;
        }
      } catch (x) {}
      return originalOpen.apply(this, arguments);
    };
  } catch (x) {}
})();
"#;

/// 图片预览小窗里注入的查看器。
///
/// 小窗直接加载图片地址，WebView2 会把这个响应渲染成一张「图片文档」。这里在
/// 文档就绪后重新排版：顶部一条 34px 的路径栏（可复制），下面是图片本体，
/// 点按钮在「适应窗口 / 原始大小」之间切换，图片没加载出来时给一句说明。
/// 占位符 `__MB_IMG_PATH__` 由宿主用 `serde_json::to_string` 换成 JS 字符串字面量，
/// 不要手写引号拼路径（路径里可能有引号、反斜杠、中文）。
pub(crate) const IMAGE_PREVIEW_JS: &str = r#"
(function () {
  var PATH_TEXT = __MB_IMG_PATH__;
  var BAR_ID = '__mb_img_bar';
  var BODY_ID = '__mb_img_body';

  var CSS = [
    'html, body { margin:0 !important; padding:0 !important; background:#14161a !important; }',
    'body { overflow:hidden !important; }',
    '#' + BAR_ID + ' { position:fixed; top:0; left:0; right:0; height:34px; z-index:2147483647;',
    '  display:flex; align-items:center; gap:8px; padding:0 10px; box-sizing:border-box;',
    '  background:#20242b; border-bottom:1px solid #333a45; color:#e6e9ee;',
    '  font:12px/1 -apple-system,"Segoe UI","Microsoft YaHei",sans-serif; }',
    '#' + BAR_ID + ' .mbp { flex:1 1 auto; overflow:hidden; white-space:nowrap; text-overflow:ellipsis; }',
    '#' + BAR_ID + ' button { flex:0 0 auto; height:22px; padding:0 9px; cursor:pointer;',
    '  border:1px solid #3d4653; border-radius:5px; background:#2b313a; color:#dfe4ea; font-size:12px; }',
    '#' + BAR_ID + ' button:hover { background:#39414d; }',
    '#' + BODY_ID + ' { position:absolute; top:34px; left:0; right:0; bottom:0; overflow:auto;',
    '  display:flex; align-items:center; justify-content:center; }',
    '#' + BODY_ID + ' img { display:block; max-width:100%; max-height:100%; }',
    '#' + BODY_ID + '.mb-original { display:block; }',
    '#' + BODY_ID + '.mb-original img { max-width:none; max-height:none; }',
    '#__mb_img_note { position:absolute; top:34px; left:0; right:0; padding:6px 10px;',
    '  background:#4a2530; color:#ffd9e0; font:12px/1.5 -apple-system,"Segoe UI",sans-serif; z-index:2147483647; }'
  ].join('\n');

  function note(text) {
    try {
      if (document.getElementById('__mb_img_note')) return;
      var d = document.createElement('div');
      d.id = '__mb_img_note';
      d.textContent = text;
      document.body.appendChild(d);
    } catch (e) {}
  }

  function copyPath(btn) {
    function legacy() {
      try {
        var ta = document.createElement('textarea');
        ta.value = PATH_TEXT;
        ta.style.cssText = 'position:fixed;top:-1000px;opacity:0';
        document.body.appendChild(ta);
        ta.select();
        var done = document.execCommand('copy');
        ta.remove();
        return done;
      } catch (e) { return false; }
    }
    function settle(text) {
      try { btn.textContent = text; } catch (e) {}
      setTimeout(function () { try { btn.textContent = '复制路径'; } catch (e) {} }, 1200);
    }
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(PATH_TEXT).then(
          function () { settle('已复制'); },
          function () { settle(legacy() ? '已复制' : '复制失败'); }
        );
        return;
      }
    } catch (e) {}
    settle(legacy() ? '已复制' : '复制失败');
  }

  function build() {
    try {
      var img = document.querySelector('img');
      // 不是图片文档（例如服务端返回了 HTML 或附件）就不动它，别把人家的页面拆了。
      if (!img) return false;
      if (document.getElementById(BAR_ID)) return true;
      document.title = PATH_TEXT;

      var style = document.createElement('style');
      style.textContent = CSS;
      (document.head || document.documentElement).appendChild(style);

      var path = document.createElement('span');
      path.className = 'mbp';
      path.textContent = PATH_TEXT;
      path.title = PATH_TEXT;

      var copyBtn = document.createElement('button');
      copyBtn.type = 'button';
      copyBtn.textContent = '复制路径';
      copyBtn.addEventListener('click', function () { copyPath(copyBtn); });

      var zoomBtn = document.createElement('button');
      zoomBtn.type = 'button';
      zoomBtn.textContent = '原始大小';

      var bar = document.createElement('div');
      bar.id = BAR_ID;
      bar.appendChild(path);
      bar.appendChild(copyBtn);
      bar.appendChild(zoomBtn);

      var body = document.createElement('div');
      body.id = BODY_ID;
      body.appendChild(img);
      zoomBtn.addEventListener('click', function () {
        var original = body.classList.toggle('mb-original');
        zoomBtn.textContent = original ? '适应窗口' : '原始大小';
      });

      document.body.appendChild(body);
      document.body.appendChild(bar);

      var warn = function () {
        if (img.naturalWidth === 0) {
          note('图片没能加载出来：链接可能已过期，或需要该账号的登录状态。路径见上方，可直接复制。');
        }
      };
      if (img.complete) setTimeout(warn, 200);
      else img.addEventListener('error', warn);
      return true;
    } catch (e) { return false; }
  }

  function boot() { if (!build()) setTimeout(build, 300); }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', boot);
  else boot();
  window.addEventListener('load', boot);
})();
"#;

/// 预览小窗的序号：同一毫秒内连点两张图也不会撞 window label。
static IMAGE_WINDOW_SEQ: AtomicU64 = AtomicU64::new(1);

pub(crate) fn query_map(url: &Url) -> HashMap<String, String> {
    url.query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

/// 路径太长时保留尾部：文件名和辨识度最高的段都在后面。
pub(crate) fn clip_tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let tail: String = text.chars().skip(count - max).collect();
    format!("…{tail}")
}

/// 百分号解码，只用于标题栏展示：中文文件名在 URL 里是 %E5%9B%BE...，
/// 直接显示没人看得懂。非法序列原样保留（`Url::path()` 给的是编码后的原始路径）。
pub(crate) fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let Some(hex) = input.get(i + 1..i + 3) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 小窗标题栏（也是窗口标题）显示什么：就是图片路径本身。
/// blob: 没有可读路径，退回链接文字（alt / title），再没有就写个通用名。
pub(crate) fn image_window_title(raw_url: &str, hint: &str) -> String {
    let hint = hint.trim();
    if raw_url.starts_with("blob:") {
        return if hint.is_empty() {
            "blob 预览图".to_string()
        } else {
            clip_tail(hint, 160)
        };
    }
    let display = match Url::parse(raw_url) {
        Ok(url) => {
            let mut out = String::new();
            out.push_str(url.scheme());
            out.push_str("://");
            if let Some(host) = url.host_str() {
                out.push_str(host);
            }
            out.push_str(&percent_decode(url.path()));
            if let Some(q) = url.query() {
                out.push('?');
                out.push_str(q);
            }
            out
        }
        Err(_) => raw_url.to_string(),
    };
    clip_tail(&display, 160)
}

fn build_image_window(
    app: &AppHandle,
    label: &str,
    url: Url,
    title: String,
    script: String,
    data_dir: Option<PathBuf>,
) -> tauri::Result<tauri::WebviewWindow> {
    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::External(url))
        .title(title)
        .inner_size(760.0, 620.0)
        .min_inner_size(320.0, 240.0)
        .resizable(true)
        .center()
        .disable_drag_drop_handler()
        .initialization_script(script);
    if let Some(dir) = data_dir {
        builder = builder.data_directory(dir);
    }
    builder.build()
}

/// 另开一个小窗显示图片本体，窗口标题就是图片路径。**不动账号页**。
///
/// 数据目录优先复用账号自己的 UserDataFolder：blob: 图片和需要 cookie 的图片
/// 只有落在同一个存储分区里才取得出来。WebView2 不允许同一目录挂两套不同的
/// 环境参数，复用失败（账号配了代理 / 语言时参数就不同）就退回默认数据目录 ——
/// 这时 http(s) 图片照常能看，只是一次性链接以外的会话内图片可能加载不出来。
fn open_image_window(app: &AppHandle, profile_id: &str, raw_url: &str, hint: &str) {
    let Ok(url) = Url::parse(raw_url) else { return };
    if !matches!(url.scheme(), "http" | "https" | "blob") {
        return;
    }
    let title = image_window_title(raw_url, hint);
    let script = IMAGE_PREVIEW_JS.replace(
        "__MB_IMG_PATH__",
        &serde_json::to_string(&title).unwrap_or_else(|_| "\"\"".to_string()),
    );
    let stamp = Utc::now().timestamp_millis();
    let seq = IMAGE_WINDOW_SEQ.fetch_add(1, Ordering::Relaxed);
    let label = format!("mb-image-{stamp}-{seq}");

    if let Ok(dir) = profile_data_dir(app, profile_id) {
        if let Ok(window) = build_image_window(
            app,
            &label,
            url.clone(),
            title.clone(),
            script.clone(),
            Some(dir),
        ) {
            let _ = window.set_focus();
            return;
        }
    }
    // label 可能已被上一次尝试占住（环境起来了、窗口没成），换个 label 再试。
    let fallback_label = format!("mb-image-{stamp}-{seq}-plain");
    if let Ok(window) = build_image_window(app, &fallback_label, url, title, script, None) {
        let _ = window.set_focus();
    }
}

pub(crate) fn fingerprint_script(id: &str) -> String {
    FINGERPRINT_GUARD_JS.replace("__SEED__", &profile_seed(id).to_string())
}

/// 注入每个账号 WebView 的状态抓取脚本：在 chat01.ai 页面里用启发式规则
/// （DOM 文本 + localStorage）提取积分和登录账号，然后通过一次会被后端
/// 取消的 mbstatus:// 假导航把数据带出来。chat01.ai 是客户端渲染的 SPA，
/// 无法依赖固定选择器，所以采取多策略扫描并只在结果变化时上报。
const STATUS_REPORTER_JS: &str = r#"
(function () {
  if (window.__mbStatusInstalled) return;
  try { Object.defineProperty(window, '__mbStatusInstalled', { value: true }); } catch (e) { return; }

  var EMAIL_RE = /^[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}$/;
  var CRED_HEAD = /(?:积分|点数|余额|credits?)\s*[:：=]?\s*([0-9][0-9,]*(?:\.[0-9]+)?)/i;
  var CRED_TAIL = /([0-9][0-9,]*(?:\.[0-9]+)?)\s*(?:个)?\s*(?:积分|点数|credits?)\b/i;
  var CRED_KEY = /credit|balance|remain|quota|points?|jifen|积分/;
  var NAME_KEY = /^(display_?name|user_?name|nick(name)?|full_?name|name)$/;
  // alt / aria-label 里常见的界面用词，不能当作登录账号名。
  var NOT_A_NAME = /^(true|false|null|undefined|user|admin|member|guest|logo|logo\.png|icon|icons|image|img|photo|picture|avatar|profile|account|menu|home|search|settings|setting|close|open|toggle|send|submit|copy|edit|delete|remove|more|back|next|previous|refresh|reload|loading|app|site|web|link|chat|chat01|chat01\.ai|gpt|ai|用户|账户|账号|头像|图标|标志|菜单|首页|搜索|设置|关闭|打开|更多|发送|复制|删除|返回|刷新|加载|加载中|登录|退出|注册)$/;
  var JUNK_NAME_RE = NOT_A_NAME;

  // 页面脚本检测 hook 的第一招就是 fetch.toString()，这里伪装成原生函数。
  function maskNative(fn, name) {
    try {
      Object.defineProperty(fn, 'toString', { value: function () { return 'function ' + name + '() { [native code] }'; }, writable: true, configurable: true });
      Object.defineProperty(fn, 'name', { value: name, writable: true, configurable: true });
    } catch (e) {}
    return fn;
  }

  function onSite() {
    var h = (location.hostname || '').toLowerCase();
    return h === 'chat01.ai' || (h.length > 10 && h.slice(-10) === '.chat01.ai');
  }

  function scanObject(obj, depth, out) {
    if (!obj || typeof obj !== 'object' || depth > 4) return;
    try {
      for (var key in obj) {
        if (!Object.prototype.hasOwnProperty.call(obj, key)) continue;
        var v = obj[key];
        if (v === null || v === undefined) continue;
        var lk = String(key).toLowerCase();
        if (typeof v === 'object') { scanObject(v, depth + 1, out); continue; }
        // 积分始终覆盖：接口返回的余额会随消耗变化，不能只取第一次。
        if (CRED_KEY.test(lk)) {
          var num = typeof v === 'number' ? v : parseFloat(String(v).replace(/[,，\s]/g, ''));
          if (isFinite(num) && num >= 0 && num < 1e9) out.credits = String(Math.round(num * 100) / 100);
        }
        if (typeof v === 'string') {
          var t = v.trim();
          if (!out.email && /e-?mail|login|account/.test(lk) && t.length < 100 && EMAIL_RE.test(t)) out.email = t;
          if (!out.name && NAME_KEY.test(lk) && t && t.length <= 60 && !EMAIL_RE.test(t) && !NOT_A_NAME.test(t)) out.name = t;
        }
      }
    } catch (e) {}
  }

  function scanStorages() {
    var out = { credits: '', name: '', email: '' };
    var stores = [];
    try { stores.push(window.localStorage); } catch (e) {}
    try { stores.push(window.sessionStorage); } catch (e) {}
    for (var si = 0; si < stores.length; si++) {
      var store = stores[si];
      var len = 0;
      try { len = store.length; } catch (e) { continue; }
      for (var i = 0; i < len && i < 80; i++) {
        var raw = null;
        try { raw = store.getItem(store.key(i)); } catch (e) {}
        if (!raw) continue;
        var parsed = null;
        try { parsed = JSON.parse(raw); } catch (e) {}
        if (parsed && typeof parsed === 'object') {
          scanObject(parsed, 0, out);
        } else if (EMAIL_RE.test(String(raw).trim())) {
          if (!out.email) out.email = String(raw).trim();
        }
      }
    }
    return out;
  }

  function scanDom() {
    var out = { credits: '', name: '', email: '' };
    try {
      var nodes = document.querySelectorAll('div,span,p,a,button,strong,b,em,i,td,th,li,label,h1,h2,h3,h4,h5,h6,small');
      var emails = [];
      for (var i = 0; i < nodes.length; i++) {
        var el = nodes[i];
        var t = (el.textContent || '').replace(/\s+/g, ' ').trim();
        if (!t || t.length > 40) continue;
        if (!out.credits) {
          var m = t.match(CRED_HEAD) || t.match(CRED_TAIL);
          if (m) out.credits = m[1].replace(/,/g, '');
        }
        var em = t.match(EMAIL_RE);
        if (em) emails.push(em[0]);
        if (el.tagName === 'A') {
          var href = el.getAttribute('href') || '';
          if (href.slice(0, 7).toLowerCase() === 'mailto:') emails.push(href.slice(7).split('?')[0]);
        }
        // 关键字段都拿到后不必把剩余节点扫完。
        if (out.credits && out.email) break;
      }
      for (var j = 0; j < emails.length; j++) {
        if (/gmail\.com$/i.test(emails[j])) { out.email = emails[j]; break; }
      }
      if (!out.email) out.email = emails[0] || '';
      var labeled = document.querySelectorAll('img[alt],button[aria-label],[aria-label]');
      for (var k = 0; k < labeled.length; k++) {
        var attr = (labeled[k].getAttribute('alt') || labeled[k].getAttribute('aria-label') || '').trim();
        if (attr && attr.length >= 2 && attr.length <= 40 && !EMAIL_RE.test(attr) &&
            /[a-z0-9\u4e00-\u9fa5]/i.test(attr) && !JUNK_NAME_RE.test(attr)) {
          out.name = attr;
          break;
        }
      }
    } catch (e) {}
    return out;
  }

  // ---- AI 回答状态：只观察 DOM 结构变化，不跟踪每个流式 token 的文字变化。
  // 这样后台同时开很多账号时，MutationObserver 的压力会明显低于 characterData 全量监听。 ----
  var profileActive = !document.hidden;
  var answerGenerating = false;
  var answerReady = false;
  var answerFinishTimer = 0;
  var answerCheckTimer = 0;

  function hasGeneratingIndicator() {
    try {
      var quick = document.querySelector(
        'button[data-testid*="stop" i],button[aria-label*="stop" i],button[title*="stop" i],button[aria-label*="停止"],button[title*="停止"]'
      );
      if (quick) return true;
      var buttons = document.querySelectorAll('button,[role="button"]');
      var limit = Math.min(buttons.length, 180);
      for (var i = 0; i < limit; i++) {
        var el = buttons[i];
        var text = String(
          el.getAttribute('aria-label') || el.getAttribute('title') || el.textContent || ''
        ).replace(/\s+/g, ' ').trim();
        if (/^(?:停止(?:生成|回答|输出|响应|思考)?|取消生成|stop(?: generating| generation| response)?|cancel generation)$/i.test(text)) {
          return true;
        }
      }
    } catch (e) {}
    return false;
  }

  function checkAnswerState() {
    answerCheckTimer = 0;
    var generating = hasGeneratingIndicator();
    if (generating) {
      if (answerFinishTimer) { clearTimeout(answerFinishTimer); answerFinishTimer = 0; }
      if (!answerGenerating) {
        answerGenerating = true;
        answerReady = false;
        try { report(true); } catch (e) {}
      }
      return;
    }
    if (!answerGenerating || answerFinishTimer) return;
    // 结束按钮偶尔会因前端重渲染短暂消失，稳定 1.2 秒后才判定回答完成。
    answerFinishTimer = setTimeout(function () {
      answerFinishTimer = 0;
      if (hasGeneratingIndicator()) { scheduleAnswerCheck(); return; }
      if (!answerGenerating) return;
      answerGenerating = false;
      // 只有后台账号才需要“回答完成”提醒；当前正在看的账号不额外打扰。
      answerReady = !profileActive;
      try { report(true); } catch (e) {}
    }, 1200);
  }

  function scheduleAnswerCheck() {
    if (answerCheckTimer) return;
    answerCheckTimer = setTimeout(checkAnswerState, 300);
  }

  try {
    document.addEventListener('mb-profile-active', function (e) {
      try {
        profileActive = !!(e.detail && e.detail.active);
        if (profileActive) kickIfVisible();
        scheduleAnswerCheck();
      } catch (x) {}
    });
    document.addEventListener('mb-answer-seen', function () {
      if (!answerReady) return;
      answerReady = false;
      try { report(true); } catch (e) {}
    });
  } catch (e) {}

  // ---- 接口响应扫描（积分精确化）：钩住 fetch / XHR，直接解析 JSON 里的积分字段，
  // 比页面文案更准、更快（在积分被渲染前就能拿到），文案扫描仅作兜底。----
  var api = { credits: '', name: '', email: '' };

  function scanApiResponse(data) {
    try {
      if (!data || typeof data !== 'object') return;
      var before = api.credits + '|' + api.email;
      scanObject(data, 0, api);
      if (api.credits + '|' + api.email !== before) kickIfVisible();
    } catch (e) {}
  }

  try {
    var originalFetch = window.fetch;
    if (originalFetch && !Object.getOwnPropertyDescriptor(originalFetch, '__mbH')) {
      var patchedFetch = function () {
        var promise = originalFetch.apply(this, arguments);
        try {
          promise
            .then(function (res) {
              try {
                var ct = '';
                try { ct = (res.headers && res.headers.get('content-type')) || ''; } catch (e) {}
                if (ct.indexOf('json') === -1 && ct.indexOf('text') === -1) return;
                res.clone().text().then(function (text) {
                  if (text && text.length < 500000 && text.charAt(0) !== '<') {
                    scanApiResponse(JSON.parse(text));
                  }
                }).catch(function () {});
              } catch (e) {}
            })
            .catch(function () {});
        } catch (e) {}
        return promise;
      };
      maskNative(patchedFetch, 'fetch');
      try { Object.defineProperty(patchedFetch, '__mbH', { value: true }); } catch (e) {}
      window.fetch = patchedFetch;
    }
  } catch (e) {}

  try {
    var xhrProto = XMLHttpRequest.prototype;
    if (xhrProto.send && !Object.getOwnPropertyDescriptor(xhrProto.send, '__mbH')) {
      var originalSend = xhrProto.send;
      var patchedSend = function () {
        try {
          this.addEventListener('load', function () {
            try {
              var text = this.responseText || '';
              if (text && text.length < 500000 && text.charAt(0) !== '<') {
                scanApiResponse(JSON.parse(text));
              }
            } catch (e) {}
          });
        } catch (e) {}
        return originalSend.apply(this, arguments);
      };
      maskNative(patchedSend, 'send');
      try { Object.defineProperty(patchedSend, '__mbH', { value: true }); } catch (e) {}
      xhrProto.send = patchedSend;
    }
  } catch (e) {}

  var lastPayload = '';
  function report(force) {
    if (!onSite()) return;
    var st = scanStorages();
    var dom = scanDom();
    var credits = api.credits || dom.credits || st.credits || '';
    var name = api.name || st.name || dom.name || '';
    var email = api.email || dom.email || st.email || '';
    // 谷歌账号的登录名就是邮箱，优先展示；页面昵称只在没有邮箱时兜底。
    var shown = email || name;
    if (!credits && !shown && !force && !answerReady && !answerGenerating) return;
    var payload = 'c=' + encodeURIComponent(credits) +
      '&n=' + encodeURIComponent(shown) +
      '&e=' + encodeURIComponent(email) +
      '&r=' + (answerReady ? '1' : '0') +
      '&g=' + (answerGenerating ? '1' : '0');
    if (!force && payload === lastPayload) return;
    lastPayload = payload;
    try { location.href = 'mbstatus://report?' + payload; } catch (e) {}
  }

  // 后端"刷新状态"按钮通过 mb-status-scan 自定义事件触发强制重扫。
  try {
    document.addEventListener('mb-status-scan', function () { try { report(true); } catch (e) {} });
  } catch (e) {}

  // ---- AI 生成文件自动下载：后端通过 mb-dl-cfg 自定义事件下发开关与扩展名白名单。
  // 修复点：配置到达时主动扫描已有链接；同时监听 href/download 属性变化，覆盖
  // React/Vue 先插入 <a>、后异步赋 href 的情况；HTTP(S) 文件链接也纳入识别。----
  var dlCfg = { auto: false, skipDownloaded: true, downloadedNames: [], blockFirstSeconds: 3, exts: [] };
  var autoScanTimer = 0;
  var downloadGuardStartedAt = Date.now();

  function normalizedExt(value) {
    var raw = String(value || '').split('#')[0].split('?')[0];
    var m = /\.([a-z0-9]{1,8})$/i.exec(raw);
    return m ? m[1].toLowerCase() : '';
  }

  function inferredFileName(name, href) {
    var rawName = String(name || '').trim();
    if (rawName) {
      rawName = rawName.split(/[\\/]/).pop() || rawName;
      if (/\.[a-z0-9]{1,8}$/i.test(rawName)) return rawName.toLowerCase();
    }
    try {
      var clean = String(href || '').split('#')[0].split('?')[0];
      var decoded = decodeURIComponent(clean);
      var part = decoded.split('/').pop() || '';
      return part.toLowerCase();
    } catch (e) { return ''; }
  }

  function alreadyDownloaded(name, href) {
    if (!dlCfg.skipDownloaded) return false;
    var candidate = inferredFileName(name, href);
    if (!candidate) return false;
    var names = dlCfg.downloadedNames || [];
    for (var i = 0; i < names.length; i++) {
      if (String(names[i] || '').toLowerCase() === candidate) return true;
    }
    return false;
  }

  function extAllowed(name, href) {
    if (!dlCfg.auto) return false;
    var exts = dlCfg.exts || [];
    if (!exts.length) return true;
    var ext = normalizedExt(name) || normalizedExt(href);
    // 白名单开启后必须能识别出扩展名；未知格式不自动下载，避免白名单被绕过。
    if (!ext) return false;
    return exts.indexOf(ext) !== -1;
  }

  function looksDownloadLike(el, href) {
    if (el.hasAttribute('download')) return true;
    if (/^(?:blob:|data:)/i.test(href)) return true;
    var clean = String(href || '').split('#')[0].split('?')[0];
    if (/\.[a-z0-9]{1,8}$/i.test(clean)) return true;
    if (/(?:^|[\/?&=_-])(?:download|export|attachment|file)(?:[\/?&=_-]|$)/i.test(href)) return true;
    var text = String(el.textContent || el.getAttribute('aria-label') || el.getAttribute('title') || '');
    return /(?:下载|导出|保存文件|download|export|save file)/i.test(text);
  }

  function downloadGuardRemainingMs() {
    var seconds = Math.max(0, Number(dlCfg.blockFirstSeconds) || 0);
    return Math.max(0, seconds * 1000 - (Date.now() - downloadGuardStartedAt));
  }

  function tryAutoDownload(el) {
    try {
      if (!onSite() || !dlCfg.auto || el.__mbAutoClicked || el.tagName !== 'A') return;
      if (downloadGuardRemainingMs() > 0) return;
      var href = el.getAttribute('href') || '';
      if (!href || href === '#' || /^javascript:/i.test(href)) return;
      if (!looksDownloadLike(el, href)) return;
      var name = el.getAttribute('download') || el.getAttribute('title') || el.textContent || '';
      if (!extAllowed(name, href)) return;
      if (alreadyDownloaded(name, href)) { el.__mbAutoClicked = true; return; }
      el.__mbAutoClicked = true;
      // target=_blank 在嵌入 WebView 中容易转成被拒绝的新窗口请求；强制当前 WebView。
      if ((el.getAttribute('target') || '').toLowerCase() === '_blank') el.removeAttribute('target');
      el.click();
    } catch (e) {}
  }

  function scanAutoDownloads(root) {
    try {
      if (!dlCfg.auto || !onSite()) return;
      if (root && root.tagName === 'A') tryAutoDownload(root);
      var base = root && root.querySelectorAll ? root : document;
      var anchors = base.querySelectorAll('a[href]');
      for (var i = 0; i < anchors.length; i++) tryAutoDownload(anchors[i]);
    } catch (e) {}
  }

  function scheduleAutoScan() {
    if (autoScanTimer) clearTimeout(autoScanTimer);
    var delay = Math.max(50, downloadGuardRemainingMs() + 50);
    autoScanTimer = setTimeout(function () {
      autoScanTimer = 0;
      scanAutoDownloads(document);
    }, delay);
  }

  try {
    document.addEventListener('mb-dl-cfg', function (e) {
      try {
        if (e.detail && typeof e.detail === 'object') dlCfg = e.detail;
        // 配置在 on_page_load 中下发；与后端同一时点重置保护起点，避免慢页面计时漂移。
        downloadGuardStartedAt = Date.now();
        installMutationObserver();
        if (dlCfg.auto) scheduleAutoScan();
      } catch (x) {}
    });
  } catch (e) {}

  function handleMutations(muts) {
    scheduleAnswerCheck();
    try {
      if (!dlCfg.auto) return;
      for (var i = 0; i < muts.length; i++) {
        var m = muts[i];
        if (m.type === 'attributes') {
          tryAutoDownload(m.target);
          continue;
        }
        var nodes = m.addedNodes || [];
        for (var j = 0; j < nodes.length; j++) {
          var n = nodes[j];
          if (n.nodeType !== 1) continue;
          scanAutoDownloads(n);
        }
      }
    } catch (e) {}
  }

  var domObserver = null;
  function installMutationObserver() {
    try {
      if (domObserver) domObserver.disconnect();
      domObserver = new MutationObserver(handleMutations);
      var options = { childList: true, subtree: true };
      // 仅在自动下载开启时才监听 href/download 属性；不再监听 characterData，
      // 避免 AI 流式输出每个 token 都触发观察器。
      if (dlCfg.auto) {
        options.attributes = true;
        options.attributeFilter = ['href', 'download', 'target'];
      }
      domObserver.observe(document.documentElement, options);
    } catch (e) {}
  }

  var pending = 0;
  function kick() {
    if (pending) return;
    pending = setTimeout(function () { pending = 0; try { report(false); } catch (e) {} }, 2500);
  }

  // 隐藏的后台账号 WebView 不做积分/账号的周期扫描；AI 完成检测仍保持轻量运行。
  function kickIfVisible() {
    if (profileActive && !document.hidden) kick();
  }

  function start() {
    if (!onSite()) return;
    setTimeout(function () { if (profileActive) { try { report(false); } catch (e) {} } }, 800);
    setTimeout(function () { if (profileActive) { try { report(false); } catch (e) {} } }, 3000);
    setInterval(function () {
      if (profileActive && !document.hidden) { try { report(false); } catch (e) {} }
    }, 12000);
    try { document.addEventListener('visibilitychange', kickIfVisible); } catch (e) {}
    installMutationObserver();
    scheduleAnswerCheck();
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }
})();
"#;

pub(crate) fn timezone_script(tz: &str) -> String {
    TIMEZONE_JS.replace("__TZ__", tz)
}

pub(crate) fn locale_script(locale: &str) -> String {
    LOCALE_JS.replace("__LOCALE__", locale)
}

/// 自定义 UA 生效时同步伪装 userAgentData。UA 里仍带 Edg/ 或解析不出
/// Chrome 版本时返回 None（保持原生，不做半吊子伪装）。
pub(crate) fn user_agent_data_script(user_agent: &str) -> Option<String> {
    if user_agent.contains("Edg/") {
        return None;
    }
    let start = user_agent.find("Chrome/")? + "Chrome/".len();
    let rest = &user_agent[start..];
    let version = rest.split([' ', ';']).next()?;
    if version.is_empty()
        || version.len() > 20
        || !version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    {
        return None;
    }
    let major = version.split('.').next()?;
    let platform = if user_agent.contains("Android") {
        "Android"
    } else if user_agent.contains("Macintosh") {
        "macOS"
    } else if user_agent.contains("Linux") && !user_agent.contains("Android") {
        "Linux"
    } else if user_agent.contains("iPhone") || user_agent.contains("iPad") {
        "iOS"
    } else {
        "Windows"
    };
    let mobile = if platform == "Android" || platform == "iOS" {
        "true"
    } else {
        "false"
    };
    let brands = serde_json::json!([
        { "brand": "Not.A/Brand", "version": "99" },
        { "brand": "Chromium", "version": major },
        { "brand": "Google Chrome", "version": major }
    ])
    .to_string();
    Some(
        USER_AGENT_DATA_JS
            .replace("__BRANDS__", &brands)
            .replace("__FULL__", version)
            .replace("__MOBILE__", mobile)
            .replace("__PLATFORM__", platform),
    )
}

/// 从代理 URL 拼出 --proxy-server 参数值。Chromium 该参数不支持内嵌用户名密码，
/// 显式丢弃 userinfo，避免带认证的代理导致参数解析失败。
pub(crate) fn proxy_endpoint(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
        None => format!("{}://{}", url.scheme(), host),
    }
}

pub(crate) fn create_profile_webview(
    app: &AppHandle,
    profile: &Profile,
    bounds: &BrowserBounds,
) -> Result<(), String> {
    let label = profile_label(&profile.id);
    if app.get_webview(&label).is_some() {
        return Ok(());
    }

    // UI 主 WebView 所在的窗口。浏览器子 WebView 只占据 BrowserViewport 的矩形区域，
    // 因而不会盖住左/右侧栏、顶部地址栏以及宿主侧快捷工具栏。
    let parent = app
        .get_window("main")
        .ok_or_else(|| "找不到 main 窗口".to_string())?;

    // 无论旧数据是否完整，都保证 WebView 有一个可用启动地址。
    let url = profile_start_url(app, profile);

    let id_for_event = profile.id.clone();
    let app_for_event = app.clone();
    let id_for_image = profile.id.clone();
    let app_for_page_load = app.clone();
    let id_for_page_load = profile.id.clone();
    let download_guard_started_at = Arc::new(Mutex::new(Instant::now()));
    let download_guard_for_navigation = download_guard_started_at.clone();
    let download_guard_for_page_load = download_guard_started_at.clone();

    let mut builder = WebviewBuilder::new(label.clone(), WebviewUrl::External(url))
        // 保留系统默认 UA。不要为了绕过 Google OAuth 风控而伪造 Chrome UA。
        .incognito(profile.incognito)
        .on_navigation(move |url| {
            // 注入脚本用 mbstatus:// 假导航回传积分 / 登录账号，这里拦截并取消导航。
            if url.scheme() == STATUS_SCHEME {
                if url.host_str() == Some("report") {
                    let query = query_map(url);
                    let field = |key: &str| {
                        query
                            .get(key)
                            .map(|v| v.trim().to_string())
                            .filter(|v| !v.is_empty())
                    };
                    save_profile_status(
                        &app_for_event,
                        &id_for_event,
                        clean_status(ProfileStatus {
                            credits: field("c"),
                            account: field("n"),
                            email: field("e"),
                            updated_at: Some(Utc::now().to_rfc3339()),
                            answer_ready: query.get("r").map(|v| v == "1").unwrap_or(false),
                            answer_generating: query.get("g").map(|v| v == "1").unwrap_or(false),
                            ..Default::default()
                        }),
                    );
                }
                return false;
            }
            // 注入脚本用 mbimage://open?u=<图片地址> 请求把图片放进独立小窗。
            // 同样取消导航：账号页的网址和位置都保持原样，用户不会丢掉当前会话。
            if url.scheme() == IMAGE_SCHEME {
                if url.host_str() == Some("open") {
                    let query = query_map(url);
                    let target = query
                        .get("u")
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty());
                    if let Some(target) = target {
                        let hint = query.get("t").cloned().unwrap_or_default();
                        let app_for_image = app_for_event.clone();
                        let id_for_image = id_for_image.clone();
                        // 建窗放到事件循环下一拍：在 NavigationStarting 回调里同步
                        // 建 WebView2 会和 WebView2 自己的消息处理打架。
                        let handle_for_image = app_for_image.clone();
                        let _ = app_for_image.run_on_main_thread(move || {
                            open_image_window(&handle_for_image, &id_for_image, &target, &hint);
                        });
                    }
                }
                return false;
            }
            if let Ok(mut started_at) = download_guard_for_navigation.lock() {
                *started_at = Instant::now();
            }
            let value = url.as_str().to_string();
            // files.chat01.ai/python-generations 只作为下载中转页，不写入账号的 last_url。
            // 即使应用在下载过程中退出，下次打开账号也不会再次落到下载地址。
            if !is_chat01_download_url(url) {
                update_last_url(&app_for_event, &id_for_event, &value);
            }
            let _ = app_for_event.emit(
                "profile:navigated",
                NavigationEvent {
                    id: id_for_event.clone(),
                    url: value,
                },
            );
            true
        })
        // 状态抓取脚本对每个账号无条件注入（不受指纹防护等开关影响）。
        .initialization_script(STATUS_REPORTER_JS)
        .initialization_script(NEW_WINDOW_PATCH_JS)
        // 创建 WebView 后立刻 push 一次可能撞上文档尚未初始化；每次页面加载
        // 都重新下发当前配置，确保刷新、登录重定向后自动下载仍然生效。
        .on_page_load(move |webview, _payload| {
            if let Ok(mut started_at) = download_guard_for_page_load.lock() {
                *started_at = Instant::now();
            }
            let _ = webview.eval(download_cfg_eval_script(&app_for_page_load));
            // 刷新前若保存了页面位置，则仅恢复一次。URL hash 同时保留，
            // 对长对话/文档页面比单纯 reload 更接近刷新前的位置。
            let _ = webview.eval(
                r#"
              try {
                var raw = sessionStorage.getItem('__mab_refresh_position_v1');
                if (raw) {
                  sessionStorage.removeItem('__mab_refresh_position_v1');
                  var pos = JSON.parse(raw);
                  var restore = function() {
                    if (pos.hash && location.hash !== pos.hash) location.hash = pos.hash;
                    window.scrollTo(Number(pos.x) || 0, Number(pos.y) || 0);
                  };
                  requestAnimationFrame(function(){ requestAnimationFrame(restore); });
                  setTimeout(restore, 350);
                }
              } catch (_) {}
            "#,
            );
            // SPA 刷新/登录重定向会重建 document，重新下发前后台状态，
            // 避免隐藏账号误以为自己在前台而恢复高频扫描。
            let _ = webview.eval(profile_activity_eval_script(is_active_profile(
                &id_for_page_load,
            )));
        });

    // 下载统一接管：所有下载静默保存到设置的下载目录，重名自动加序号，
    // 完成后通过事件通知前端弹提示。chat01 AI 文件的“自动点击下载”
    // 由注入脚本负责，这里负责落盘。
    let app_for_download = app.clone();
    let id_for_download = profile.id.clone();
    let download_guard_for_download = download_guard_started_at.clone();
    // 记住当前下载来源。所有 files.chat01.ai/python-generations 直链在完成后回主界面。
    // 另外用独立、不可随历史清理的下载账本从后端硬性阻止重复下载。
    let pending_download_url = Arc::new(Mutex::new(None::<String>));
    let queued_downloads = Arc::new(Mutex::new(HashSet::<String>::new()));
    let queued_downloads_for_download = queued_downloads.clone();
    builder = builder.on_download(move |webview, event| match event {
        tauri::webview::DownloadEvent::Requested { url, destination } => {
            let settings = load_app_settings(&app_for_download);
            let guard_seconds = download_guard_seconds_for(&settings, &url);
            let elapsed = download_guard_for_download.lock().ok().map(|started_at| started_at.elapsed()).unwrap_or_default();
            let guard_active = guard_seconds > 0 && elapsed < Duration::from_secs(guard_seconds);
            if guard_active {
                let remaining = Duration::from_secs(guard_seconds).saturating_sub(elapsed);
                let retry_key = normalized_download_url(&url);
                let should_queue = settings.download_guard_auto_retry
                    && queued_downloads_for_download.lock().ok().map(|mut q| q.insert(retry_key.clone())).unwrap_or(false);
                let _ = app_for_download.emit("profile:download-guard-blocked", DownloadGuardBlockedEvent { seconds: remaining.as_secs().max(1), queued: should_queue });
                if should_queue {
                    let app_retry = app_for_download.clone();
                    let label_retry = profile_label(&id_for_download);
                    let queued_retry = queued_downloads_for_download.clone();
                    let retry_url = url.as_str().to_string();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(remaining + Duration::from_millis(150)).await;
                        if let Some(wv) = app_retry.get_webview(&label_retry) {
                            if let Ok(js_url) = serde_json::to_string(&retry_url) {
                                let script = format!("try{{var a=document.createElement('a');a.href={};a.download='';a.style.display='none';document.documentElement.appendChild(a);a.click();a.remove();}}catch(e){{}}", js_url);
                                let _ = wv.eval(&script);
                            }
                        }
                        if let Ok(mut q) = queued_retry.lock() { q.remove(&retry_key); }
                    });
                }
                return false;
            }
            let should_return_home = is_chat01_download_url(&url);
            let suggested_name = destination
                .file_name()
                .map(|name| name.to_string_lossy().trim().to_ascii_lowercase())
                .filter(|name| !name.is_empty());
            let ledger = load_download_ledger(&app_for_download);
            let duplicate = download_seen(&app_for_download, &url)
                || suggested_name.as_ref().map(|name| ledger.names.contains(name)).unwrap_or(false);
            if duplicate {
                let name = suggested_name
                    .or_else(|| inferred_download_name(&url))
                    .unwrap_or_else(|| "该文件".to_string());
                let _ = app_for_download.emit("profile:download-blocked", name);
                if should_return_home {
                    if let Ok(home) = Url::parse(DOWNLOAD_RETURN_URL) { let _ = webview.navigate(home); }
                }
                return false;
            }
            if let Ok(mut pending) = pending_download_url.lock() { *pending = Some(url.as_str().to_string()); }
            download_destination(&app_for_download, &url, destination);
            true
        }
        tauri::webview::DownloadEvent::Finished { path, success, .. } => {
            let source_url = pending_download_url.lock().ok().and_then(|mut pending| pending.take());
            let should_return_home = source_url.as_deref()
                .and_then(|raw| Url::parse(raw).ok())
                .map(|url| is_chat01_download_url(&url))
                .unwrap_or(false);
            record_download(
                &app_for_download,
                &id_for_download,
                path.map(|p| p.to_string_lossy().to_string()),
                source_url,
                success,
            );
            if should_return_home {
                if let Ok(home) = Url::parse(DOWNLOAD_RETURN_URL) { let _ = webview.navigate(home); }
            }
            true
        }
        _ => true,
    });

    // 拖拽上传：Tauri 默认接管 WebView 的拖放事件（供宿主 onDragDrop 回调使用），
    // 页面因此收不到 HTML5 drag 事件，无法把文件拖进网站的上传区。
    // 账号页面不需要宿主感知拖放，关闭接管后交给 WebView2 原生处理。
    builder = builder.disable_drag_drop_handler();

    // 新窗口请求：wry 未设置处理器时一律静默拒绝 target=_blank / window.open，
    // chat01 等站点的下载链接（新窗口打开）点击后毫无反应。这里把请求的 URL
    // 导航到当前账号页内打开（相当于“在本标签页打开”），blob:/data: 已由
    // NEW_WINDOW_PATCH_JS 在页面内接管，不会走到这里。
    let app_for_new_window = app.clone();
    let id_for_new_window = profile.id.clone();
    builder = builder.on_new_window(move |url, _features| {
        if matches!(url.scheme(), "http" | "https") {
            if let Some(webview) =
                app_for_new_window.get_webview(&profile_label(&id_for_new_window))
            {
                let _ = webview.navigate(url);
            }
        }
        NewWindowResponse::Deny
    });

    if !profile.user_agent.is_empty() {
        builder = builder.user_agent(profile.user_agent.trim());
        // UA 声称是 Chrome 时同步覆盖 userAgentData，防 Client Hints 穿帮。
        if let Some(script) = user_agent_data_script(profile.user_agent.trim()) {
            builder = builder.initialization_script(script);
        }
    }

    // 指纹防护脚本在任何页面脚本执行前注入。
    if profile.fingerprint_guard {
        builder = builder.initialization_script(fingerprint_script(&profile.id));
    }

    if !profile.timezone.is_empty() {
        builder = builder.initialization_script(timezone_script(&profile.timezone));
    }

    if !profile.locale.is_empty() {
        builder = builder.initialization_script(locale_script(&profile.locale));
    }

    let proxy = parse_proxy(&profile.proxy)?;
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    let _ = &proxy;
    let locale = profile.locale.trim().to_string();

    // Windows / WebView2：data_directory 会映射到独立的 UserDataFolder，
    // Cookie / LocalStorage / IndexedDB / Cache 因此物理分目录。
    // WebView2 环境按 UserDataFolder 隔离，即使隐身账号也分配独立目录，
    // 避免多个账号共享默认环境导致参数冲突。
    #[cfg(target_os = "windows")]
    {
        let data_dir = profile_data_dir(app, &profile.id)?;
        fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
        builder = builder.data_directory(data_dir);

        // 代理 / 语言 / WebRTC 策略统一通过 additional_browser_args 下发。
        // 注意：一旦传入该参数，wry 就不再拼自己的默认参数，所以默认的
        // --disable-features 与 autoplay 策略必须在这里原样带上。
        if proxy.is_some() || !locale.is_empty() {
            let mut args = String::from(
                "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required",
            );
            if let Some(url) = &proxy {
                args.push_str(&format!(" --proxy-server={}", proxy_endpoint(url)));
                // WebRTC 禁止非代理 UDP，防止真实 IP 绕过代理直连形成关联。
                args.push_str(" --force-webrtc-ip-handling-policy=disable_non_proxied_udp");
            }
            if !locale.is_empty() {
                args.push_str(&format!(" --lang={locale}"));
            }
            builder = builder.additional_browser_args(&args);
        } else if let Some(url) = proxy {
            builder = builder.proxy_url(url);
        }
    }

    // macOS / WKWebView：不支持 data_directory。Tauri 2.9+ 提供 data_store_identifier，
    // 对应 WKWebsiteDataStore(identifier:)；仅 macOS 14+ 支持持久化自定义数据存储。
    #[cfg(target_os = "macos")]
    {
        let uuid = Uuid::parse_str(&profile.id).map_err(|e| e.to_string())?;
        builder = builder.data_store_identifier(*uuid.as_bytes());
    }

    // 每账号独立代理（Linux 走 wry 内置支持；代理在 Windows 上已于上方组合进参数）。
    #[cfg(target_os = "linux")]
    if let Some(url) = proxy {
        builder = builder.proxy_url(url);
    }

    parent
        .add_child(
            builder,
            LogicalPosition::new(bounds.x, bounds.y),
            LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0)),
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}
