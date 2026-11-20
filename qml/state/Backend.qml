pragma Singleton
import QtQuick
import Quickshell

// The one HTTP client for the OmaFinances backend.
//
// Every request from the UI goes through request() below, so the port override (OMAFINANCES_PORT)
// and the per-launch session token (OMAFINANCES_TOKEN) live in one place. Never construct an
// XMLHttpRequest anywhere else in qml/ (`rg XMLHttpRequest qml/` must list only this file).
QtObject {
  id: root

  // OMAFINANCES_PORT lets a scratch instance talk to its own backend; the default is the real one.
  readonly property string port: Quickshell.env("OMAFINANCES_PORT") || "27183"
  readonly property string baseUrl: "http://127.0.0.1:" + port
  // The backend refuses (401) every request without it. Set by the launcher; never logged or shown.
  readonly property string _token: Quickshell.env("OMAFINANCES_TOKEN") || ""

  // Headers every request carries: the session token, then the caller's own.
  function _prepare(xhr, headers) {
    if (_token !== "") xhr.setRequestHeader("Authorization", "Bearer " + _token)
    if (!headers) return
    for (var name in headers)
      if (headers.hasOwnProperty(name)) xhr.setRequestHeader(name, headers[name])
  }

  // Asynchronous request.
  //   method   "GET" | "POST" | ...
  //   path     backend path starting with "/", query string included (callers encode their values)
  //   onDone   optional function(xhr), called once when the request has finished;
  //            xhr.status 0 means the backend did not answer
  //   body     optional string sent as is
  //   headers  optional { name: value }
  // Returns the XMLHttpRequest (for abort()).
  // DELETE and POST with a body go through here too: pass the method and, for a body, a string.
  function request(method, path, onDone, body, headers) {
    var xhr = new XMLHttpRequest()
    xhr.onreadystatechange = function() {
      if (xhr.readyState !== XMLHttpRequest.DONE) return
      if (onDone) onDone(xhr)
    }
    xhr.open(method, root.baseUrl + path, true)
    _prepare(xhr, headers)
    if (body === undefined || body === null) xhr.send()
    else xhr.send(body)
    return xhr
  }

  // request() with a JSON body: `obj` is serialized and sent with Content-Type application/json.
  function json(method, path, obj, onDone) {
    return request(method, path, onDone, JSON.stringify(obj), { "Content-Type": "application/json" })
  }
}
