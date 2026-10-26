// Persistent AX references: never equate a process ID or window title with a target.
import Cocoa
import ApplicationServices

struct Failure: Error { let message: String }
func refuse(_ message: String) throws -> Never { throw Failure(message: message) }
func attr(_ element: AXUIElement, _ name: String) throws -> CFTypeRef {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success, let value else {
        try refuse("Target does not expose " + name + "; use Copy text")
    }
    return value
}
struct Snapshot {
    let pid: pid_t
    let window: AXUIElement
    let control: AXUIElement
    let value: String
    let range: CFRange
    func matches(_ other: Snapshot) -> Bool {
        pid == other.pid && CFEqual(window, other.window) && CFEqual(control, other.control)
        && value == other.value && range.location == other.range.location && range.length == other.range.length
    }
}
func snapshot() throws -> Snapshot {
    guard AXIsProcessTrusted() else { try refuse("Grant Accessibility to Speechify Electron Candidate") }
    guard let app = NSWorkspace.shared.frontmostApplication else { try refuse("No focused application") }
    let application = AXUIElementCreateApplication(app.processIdentifier)
    AXUIElementSetMessagingTimeout(application, 1)
    let windowValue = try attr(application, kAXFocusedWindowAttribute)
    let controlValue = try attr(application, kAXFocusedUIElementAttribute)
    guard CFGetTypeID(windowValue) == AXUIElementGetTypeID(), CFGetTypeID(controlValue) == AXUIElementGetTypeID() else {
        try refuse("No accessible focused window/control")
    }
    let window = windowValue as! AXUIElement, control = controlValue as! AXUIElement
    if (try? attr(control, kAXSubroleAttribute) as? String) == "AXSecureTextField" { try refuse("Secure text target refused") }
    guard let value = try attr(control, kAXValueAttribute) as? String, value.utf16.count <= 2_000_000 else {
        try refuse("Target text cannot be safely inspected")
    }
    let rangeValue = try attr(control, kAXSelectedTextRangeAttribute)
    guard CFGetTypeID(rangeValue) == AXValueGetTypeID() else { try refuse("Target caret unavailable") }
    var range = CFRange(location: 0, length: 0)
    guard AXValueGetValue(rangeValue as! AXValue, .cfRange, &range), range.location >= 0,
          range.length >= 0, range.location <= value.utf16.count, range.length <= value.utf16.count - range.location else { try refuse("Invalid target caret") }
    return Snapshot(pid: app.processIdentifier, window: window, control: control, value: value, range: range)
}
struct Target { let token: String; let state: Snapshot }
struct Insertion { let state: Snapshot; let start: Int; let text: String }
var targets: [Target] = []
var insertion: Insertion?
func checked(_ token: String) throws -> Snapshot {
    guard let target = targets.first(where: { $0.token == token }), target.state.matches(try snapshot()) else {
        try refuse("Focus, caret or text changed; saved without pasting")
    }
    return target.state
}
func pasteKeys() throws {
    guard let source = CGEventSource(stateID: .hidSystemState) else { try refuse("Keyboard event source unavailable") }
    for code: CGKeyCode in [54,55,56,58,59,60,61,62] {
        let event = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false)
        event?.flags = []; event?.post(tap: .cghidEventTap)
    }
    for down in [true, false] {
        let event = CGEvent(keyboardEventSource: source, virtualKey: 9, keyDown: down)
        event?.flags = .maskCommand; event?.post(tap: .cghidEventTap)
    }
}
func process(_ request: [String: Any]) throws -> [String: Any] {
    switch request["type"] as? String {
    case "focus":
        let current = try snapshot()
        if let existing = targets.first(where: { $0.state.matches(current) }) { return ["target": existing.token] }
        let token = UUID().uuidString
        targets.append(Target(token: token, state: current)); if targets.count > 8 { targets.removeFirst() }
        return ["target": token]
    case "paste":
        insertion = nil
        let before = try checked(request["target"] as? String ?? "")
        let text = request["text"] as? String ?? ""
        let expected = (before.value as NSString).replacingCharacters(in: NSRange(location: before.range.location, length: before.range.length), with: text)
        try pasteKeys()
        // Acknowledgment is based on the accessible value and caret, never on a timer alone.
        for _ in 0..<20 {
            Thread.sleep(forTimeInterval: 0.025)
            if let after = try? snapshot(), after.pid == before.pid, CFEqual(after.window, before.window),
               CFEqual(after.control, before.control), after.value == expected,
               after.range.location == before.range.location + text.utf16.count, after.range.length == 0 {
                insertion = Insertion(state: after, start: before.range.location, text: text)
                return ["replaceable": true]
            }
        }
        try refuse("Paste could not be acknowledged by the target; text is saved in History")
    case "replace":
        guard let owned = insertion else { try refuse("Latest insertion could not be verified; correction saved only") }
        insertion = nil // one attempt; no retry can overwrite a later edit
        guard owned.state.matches(try snapshot()) else { try refuse("Target, caret or text changed; correction saved only") }
        let text = request["text"] as? String ?? ""
        var settable: DarwinBoolean = false
        guard AXUIElementIsAttributeSettable(owned.state.control, kAXSelectedTextAttribute as CFString, &settable) == .success,
              settable.boolValue else { try refuse("Target does not support safe replacement; correction saved only") }
        var range = CFRange(location: owned.start, length: owned.text.utf16.count)
        guard let selection = AXValueCreate(.cfRange, &range),
              AXUIElementSetAttributeValue(owned.state.control, kAXSelectedTextRangeAttribute as CFString, selection) == .success else {
            try refuse("Cannot select owned dictation; correction saved only")
        }
        // Recheck after selection, then write to that exact AX object, never to the foreground keystroke sink.
        let selected = try snapshot()
        guard selected.pid == owned.state.pid, CFEqual(selected.window, owned.state.window),
              CFEqual(selected.control, owned.state.control), selected.value == owned.state.value,
              selected.range.location == range.location, selected.range.length == range.length else {
            try refuse("Target changed during replacement; no text written")
        }
        guard AXUIElementSetAttributeValue(selected.control, kAXSelectedTextAttribute as CFString, text as CFString) == .success else {
            try refuse("Target refused replacement; correction saved only")
        }
        let expected = (owned.state.value as NSString).replacingCharacters(in: NSRange(location: owned.start, length: owned.text.utf16.count), with: text)
        guard let value = try attr(selected.control, kAXValueAttribute) as? String, value == expected else {
            try refuse("Replacement acknowledgment failed; inspect target before making further edits")
        }
        return ["replaced": true]
    case "clear": insertion = nil; targets = []; return [:]
    default: try refuse("Unknown focus operation")
    }
}
while let line = readLine() {
    var response: [String: Any] = [:]
    do {
        let request = try JSONSerialization.jsonObject(with: Data(line.utf8)) as! [String: Any]
        response["id"] = request["id"]
        response.merge(try process(request)) { _, new in new }
    } catch { response["error"] = (error as? Failure)?.message ?? "Accessibility operation failed" }
    if let data = try? JSONSerialization.data(withJSONObject: response), let line = String(data: data, encoding: .utf8) {
        print(line); fflush(stdout)
    }
}
