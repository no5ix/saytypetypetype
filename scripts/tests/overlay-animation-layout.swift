// macOS regression check; briefly displays test pills without recording audio.
// Run: swift scripts/tests/overlay-animation-layout.swift
import AppKit
import QuartzCore

final class AnimationPivot: NSView {
    override func hitTest(_ point: NSPoint) -> NSView? {
        if isHidden { return nil }
        let local = convert(point, from: superview)
        for child in subviews.reversed() {
            if let hit = child.hitTest(local) { return hit }
        }
        return nil
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

func checkAnimation(_ glass: NSView, name: String) {
    let window = NSWindow(contentRect: NSRect(x: 180, y: 180, width: 720, height: 300),
                          styleMask: .borderless, backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.isOpaque = false
    window.backgroundColor = .clear
    window.ignoresMouseEvents = true
    defer { window.orderOut(nil); window.close() }
    let root = window.contentView!
    let host = NSView()
    host.wantsLayer = true
    host.translatesAutoresizingMaskIntoConstraints = false
    root.addSubview(host)
    let width = host.widthAnchor.constraint(equalToConstant: 160)
    let height = host.heightAnchor.constraint(equalToConstant: 40)
    let pivot = AnimationPivot()
    pivot.wantsLayer = true
    pivot.clipsToBounds = false
    pivot.translatesAutoresizingMaskIntoConstraints = false
    host.addSubview(pivot)
    glass.wantsLayer = true
    glass.translatesAutoresizingMaskIntoConstraints = false
    pivot.addSubview(glass)
    NSLayoutConstraint.activate([
        host.centerXAnchor.constraint(equalTo: root.centerXAnchor),
        host.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -28), width, height,
        pivot.centerXAnchor.constraint(equalTo: host.centerXAnchor),
        pivot.centerYAnchor.constraint(equalTo: host.centerYAnchor),
        pivot.widthAnchor.constraint(equalToConstant: 0),
        pivot.heightAnchor.constraint(equalToConstant: 0),
        glass.centerXAnchor.constraint(equalTo: pivot.centerXAnchor),
        glass.centerYAnchor.constraint(equalTo: pivot.centerYAnchor),
        glass.widthAnchor.constraint(equalTo: host.widthAnchor),
        glass.heightAnchor.constraint(equalTo: host.heightAnchor),
    ])
    root.layoutSubtreeIfNeeded()
    let button = NSButton(title: "Retry", target: nil, action: nil)
    button.frame = NSRect(x: 10, y: 8, width: 70, height: 24)
    if #available(macOS 26, *), let liquid = glass as? NSGlassEffectView {
        let content = NSView(frame: glass.bounds)
        content.addSubview(button)
        liquid.contentView = content
        liquid.cornerRadius = 20
    } else { glass.addSubview(button) }
    let layer = pivot.layer!
    func animate(from: Double, to: Double, duration: Double, keep: Bool = false) {
        let animation = CABasicAnimation(keyPath: "transform.scale")
        animation.fromValue = from
        animation.toValue = to
        animation.duration = duration
        animation.fillMode = .forwards
        animation.isRemovedOnCompletion = !keep
        layer.add(animation, forKey: "visibility-scale")
    }
    func advance(_ duration: Double) {
        RunLoop.current.run(until: Date().addingTimeInterval(duration))
    }
    func checkDisplayedCenter() {
        precondition(glass.layer!.superlayer === layer, "display detached the animated view")
        precondition(layer.anchorPoint == .zero, "unexpected AppKit anchor")
        precondition(pivot.frame.midX == host.bounds.midX && pivot.frame.midY == host.bounds.midY)
        let presented = glass.layer!.presentation()!
        let center = presented.convert(CGPoint(x: presented.bounds.midX, y: presented.bounds.midY),
                                       to: root.layer!.presentation()!)
        precondition(abs(center.x - root.bounds.midX) < 0.75, "displayed pill drifted horizontally")
        precondition(abs(center.y - host.frame.midY) < 0.75, "displayed pill drifted vertically")
    }
    animate(from: 0.8, to: 1, duration: 0.16)
    window.orderFrontRegardless()
    advance(0.03)
    checkDisplayedCenter()
    let entryScale = layer.presentation()!.value(forKeyPath: "transform.scale") as! Double
    precondition(entryScale > 0.8 && entryScale < 1, "entry scale was not rendered")
    width.constant = 240
    height.constant = 80
    root.layoutSubtreeIfNeeded()
    advance(0.03)
    checkDisplayedCenter()
    advance(0.15)
    let point = root.convert(NSPoint(x: button.bounds.midX, y: button.bounds.midY), from: button)
    precondition(root.hitTest(point) === button, "zero-size pivot blocked retry clicks")
    animate(from: 1, to: 0.8, duration: 0.12, keep: true)
    advance(0.03)
    checkDisplayedCenter()
    let exitScale = layer.presentation()!.value(forKeyPath: "transform.scale") as! Double
    precondition(exitScale > 0.8 && exitScale < 1, "exit scale was not rendered")
    // Reopen during exit: replace the animation from its displayed scale.
    animate(from: exitScale, to: 1, duration: 0.16)
    advance(0.03)
    checkDisplayedCenter()
    let reopened = layer.presentation()!.value(forKeyPath: "transform.scale") as! Double
    precondition(reopened > exitScale && reopened < 1, "reopening did not reverse exit")
    print("\(name): rendered entry/exit, centered resizing, reopening and retry hit test passed")
}

checkAnimation(NSVisualEffectView(), name: "Vibrancy")
if #available(macOS 26, *) { checkAnimation(NSGlassEffectView(), name: "Liquid Glass") }
