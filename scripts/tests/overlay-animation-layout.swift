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
        pivot.bottomAnchor.constraint(equalTo: host.bottomAnchor),
        pivot.widthAnchor.constraint(equalToConstant: 0),
        pivot.heightAnchor.constraint(equalToConstant: 0),
        glass.centerXAnchor.constraint(equalTo: pivot.centerXAnchor),
        glass.bottomAnchor.constraint(equalTo: pivot.bottomAnchor),
        glass.widthAnchor.constraint(equalTo: host.widthAnchor),
        glass.heightAnchor.constraint(equalTo: host.heightAnchor),
    ])
    root.layoutSubtreeIfNeeded()
    let button = NSButton(title: "Retry", target: nil, action: nil)
    button.frame = NSRect(x: 10, y: 8, width: 70, height: 24)
    let content = NSView(frame: glass.bounds)
    content.wantsLayer = true
    content.addSubview(button)
    let label = NSTextField(labelWithString: "识别中")
    label.wantsLayer = true
    label.alignment = .center
    label.font = .systemFont(ofSize: 14)
    content.addSubview(label)
    let indicator = NSView()
    indicator.wantsLayer = true
    content.addSubview(indicator)
    if #available(macOS 26, *), let liquid = glass as? NSGlassEffectView {
        liquid.contentView = content
        liquid.cornerRadius = 20
    } else { glass.addSubview(content) }
    let layer = pivot.layer!
    func animate(from: Double, to: Double, duration: Double, keep: Bool = false) {
        let animation = CABasicAnimation(keyPath: "opacity")
        animation.fromValue = from
        animation.toValue = to
        animation.duration = duration
        animation.fillMode = .forwards
        animation.isRemovedOnCompletion = !keep
        layer.add(animation, forKey: "visibility-opacity")
    }
    func advance(_ duration: Double) {
        RunLoop.current.run(until: Date().addingTimeInterval(duration))
    }
    func checkDisplayedCenter() {
        precondition(glass.layer!.superlayer === layer, "display detached the animated view")
        precondition(layer.anchorPoint == .zero, "unexpected AppKit anchor")
        precondition(pivot.frame.midX == host.bounds.midX && pivot.frame.minY == host.bounds.minY)
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
    let entryScale = layer.presentation()!.value(forKeyPath: "opacity") as! Double
    precondition(entryScale > 0.8 && entryScale < 1, "entry opacity was not rendered")
    width.constant = 240
    height.constant = 80
    root.layoutSubtreeIfNeeded()
    advance(0.03)
    checkDisplayedCenter()
    advance(0.15)
    // Resize the persistent surface using the same bottom-center constraints
    // and real-view frame updates as the production renderer.
    var resizeTimer: Timer?
    defer { resizeTimer?.invalidate() }
    func resize(_ w: CGFloat, _ h: CGFloat) {
        resizeTimer?.invalidate()
        let from = host.frame.size
        let started = ProcessInfo.processInfo.systemUptime
        func tick() {
            let t = min((ProcessInfo.processInfo.systemUptime - started) / 0.2, 1)
            let progress = 1 - pow(1 - t, 3)
            let currentW = from.width + (w - from.width) * progress
            let currentH = from.height + (h - from.height) * progress
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0
                context.allowsImplicitAnimation = false
                width.constant = currentW
                height.constant = currentH
                root.layoutSubtreeIfNeeded()
                content.frame = glass.bounds
                label.frame = NSRect(x: 16, y: (currentH - 18) / 2, width: currentW - 32, height: 18)
                indicator.frame = NSRect(x: (currentW - 16) / 2, y: (currentH - 16) / 2, width: 16, height: 16)
                root.layer!.layoutIfNeeded()
            }
            if t == 1 { resizeTimer?.invalidate() }
        }
        tick()
        resizeTimer = Timer.scheduledTimer(withTimeInterval: 0.016, repeats: true) { _ in tick() }
    }
    func displayedRect() -> CGRect {
        let presented = glass.layer!.presentation()!
        return presented.convert(presented.bounds, to: root.layer!.presentation()!)
    }
    func checkBottomAnchor() {
        let rect = displayedRect()
        precondition(abs(rect.midX - root.bounds.midX) < 0.75, "resize drifted horizontally")
        precondition(abs(rect.minY - 28) < 0.75, "resize moved the bottom anchor")
        // Inspect the full-size internal surfaces, not just glass.layer's bounds.
        for internalLayer in glass.layer!.sublayers ?? [] {
            if internalLayer.bounds.width < 40 { continue }
            let presented = internalLayer.presentation() ?? internalLayer
            precondition(abs(presented.bounds.width - rect.width) < 0.75,
                         "internal glass width jumped ahead of the visible frame")
            precondition(abs(presented.bounds.height - rect.height) < 0.75,
                         "internal glass height jumped ahead of the visible frame")
            let internalRect = presented.convert(presented.bounds, to: root.layer!.presentation()!)
            precondition(abs(internalRect.midX - rect.midX) < 0.75, "internal glass drifted horizontally")
            precondition(abs(internalRect.minY - rect.minY) < 0.75, "internal glass drifted vertically")
        }
        for view in [label, indicator] {
            let presented = view.layer!.presentation()!
            let frame = presented.convert(presented.bounds, to: root.layer!.presentation()!)
            precondition(abs(frame.midX - rect.midX) < 0.75, "new centered content slid from its previous position")
        }
    }
    resize(480, 104)
    for _ in 0..<3 { advance(0.01); checkBottomAnchor() }
    checkBottomAnchor()
    let growing = displayedRect()
    precondition(growing.width > 240 && growing.width < 480, "resize did not interpolate width")
    precondition(growing.height > 80 && growing.height < 104, "resize did not interpolate height")
    resize(120, 40)
    advance(0.03)
    checkBottomAnchor()
    let shrinking = displayedRect()
    precondition(shrinking.width < growing.width && shrinking.width > 120, "interrupted resize jumped")
    for _ in 0..<23 { advance(0.01); checkBottomAnchor() }
    precondition(abs(displayedRect().width - 120) < 0.75, "resize missed final width")
    // The centered test content overlaps this historical retry-button fixture.
    label.isHidden = true
    indicator.isHidden = true
    let point = root.convert(NSPoint(x: button.bounds.midX, y: button.bounds.midY), from: button)
    precondition(root.hitTest(point) === button, "zero-size pivot blocked retry clicks")
    animate(from: 1, to: 0.8, duration: 0.12, keep: true)
    advance(0.03)
    checkDisplayedCenter()
    let exitScale = layer.presentation()!.value(forKeyPath: "opacity") as! Double
    precondition(exitScale > 0.8 && exitScale < 1, "exit opacity was not rendered")
    // Reopen during exit: replace the animation from its displayed opacity.
    animate(from: exitScale, to: 1, duration: 0.16)
    advance(0.03)
    checkDisplayedCenter()
    let reopened = layer.presentation()!.value(forKeyPath: "opacity") as! Double
    precondition(reopened > exitScale && reopened < 1, "reopening did not reverse exit")
    print("\(name): rendered entry/exit, bottom-anchored resizing, interrupted resizing, reopening and retry hit test passed")
}

checkAnimation(NSVisualEffectView(), name: "Vibrancy")
if #available(macOS 26, *) { checkAnimation(NSGlassEffectView(), name: "Liquid Glass") }
