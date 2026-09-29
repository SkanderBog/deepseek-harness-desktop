(function () {
  if (window.__dsh_promise_with_resolvers__) return;
  if (typeof Promise === 'undefined' || typeof Promise.withResolvers === 'function') return;
  window.__dsh_promise_with_resolvers__ = true;

  var withResolvers = function () {
    var resolve;
    var reject;
    var promise = new Promise(function (res, rej) {
      resolve = res;
      reject = rej;
    });
    return { promise: promise, resolve: resolve, reject: reject };
  };

  try {
    Object.defineProperty(Promise, 'withResolvers', {
      configurable: true,
      writable: true,
      value: withResolvers,
    });
  } catch (_e) {
    try {
      Promise.withResolvers = withResolvers;
    } catch (_ignored) { /* 保留原生未定义的现状 */ }
  }
})();
