# Runs the dev app with WebView2's DevTools port open (localhost only) so the
# window can be inspected and screenshotted with Playwright. Dev use only.
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9222"
npm run tauri dev
