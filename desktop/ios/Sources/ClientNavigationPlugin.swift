import Foundation
import Tauri
import WebKit

private struct NavigationConfig: Decodable {
    let failureURL: URL
}

// WKWebView's delegate is weak. The plugin retains the proxy, which retains Wry's
// delegate. The proxy's webview reference is weak; Wry retains its existing
// runtime ownership. All unhandled delegate APIs stay Wry's.
private final class NavigationProxy: NSObject, WKNavigationDelegate {
    private let original: WKNavigationDelegate
    private let failureURL: URL
    private weak var webview: WKWebView?
    private var candidate: URL?
    private var current: WKNavigation?
    private var deadline: DispatchWorkItem?

    init(webview: WKWebView, original: WKNavigationDelegate, failureURL: URL) {
        self.webview = webview
        self.original = original
        self.failureURL = failureURL
        super.init()
    }

    override func responds(to selector: Selector!) -> Bool {
        super.responds(to: selector) || original.responds(to: selector)
    }

    override func forwardingTarget(for selector: Selector!) -> Any? {
        original.responds(to: selector) ? original : super.forwardingTarget(for: selector)
    }

    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                 decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        if action.targetFrame?.isMainFrame == true {
            candidate = action.request.url
        }
        if original.responds(to: NSSelectorFromString("webView:decidePolicyForNavigationAction:decisionHandler:")) {
            original.webView?(webView, decidePolicyFor: action, decisionHandler: decisionHandler)
        } else {
            decisionHandler(.allow)
        }
    }

    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        deadline?.cancel()
        current = navigation
        if let navigation = navigation,
           let scheme = candidate?.scheme, ["http", "https"].contains(scheme) {
            let task = DispatchWorkItem { [weak self, weak navigation] in
                guard let self = self, let navigation = navigation,
                      self.current === navigation else { return }
                self.recover(navigation)
            }
            deadline = task
            DispatchQueue.main.asyncAfter(deadline: .now() + 30, execute: task)
        }
        original.webView?(webView, didStartProvisionalNavigation: navigation)
    }

    func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
        if current === navigation { deadline?.cancel() }
        original.webView?(webView, didCommit: navigation)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        if current === navigation { deadline?.cancel(); current = nil }
        original.webView?(webView, didFinish: navigation)
    }

    private func failed(_ navigation: WKNavigation?, error: Error) {
        guard let navigation = navigation, current === navigation else { return }
        let native = error as NSError
        if native.domain == NSURLErrorDomain && native.code == NSURLErrorCancelled {
            deadline?.cancel()
            current = nil
            return
        }
        recover(navigation)
    }

    private func recover(_ navigation: WKNavigation) {
        guard current === navigation else { return }
        deadline?.cancel()
        current = nil
        guard let scheme = candidate?.scheme, ["http", "https"].contains(scheme) else { return }
        webview?.load(URLRequest(url: failureURL))
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        original.webView?(webView, didFailProvisionalNavigation: navigation, withError: error)
        failed(navigation, error: error)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        original.webView?(webView, didFail: navigation, withError: error)
        failed(navigation, error: error)
    }
}

final class ClientNavigationPlugin: Plugin {
    private var proxies: [NavigationProxy] = []

    override func load(webview: WKWebView) {
        guard let config = try? parseConfig(NavigationConfig.self),
              let original = webview.navigationDelegate else { return }
        let proxy = NavigationProxy(webview: webview, original: original, failureURL: config.failureURL)
        proxies.append(proxy)
        webview.navigationDelegate = proxy
    }
}

@_cdecl("init_plugin_client_navigation")
public func initClientNavigationPlugin() -> Plugin {
    ClientNavigationPlugin()
}
