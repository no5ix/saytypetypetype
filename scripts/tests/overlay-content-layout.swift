// Native regression check for fixed shortcut positioning and glass foreground inheritance.
// Run: swift scripts/tests/overlay-content-layout.swift
import AppKit
import QuartzCore

final class AdaptiveContent: NSView {
    var appearanceChanged: (() -> Void)?
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        appearanceChanged?()
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

func advance(_ duration: Double) {
    RunLoop.current.run(until: Date().addingTimeInterval(duration))
}

func checkContent(_ glass: NSView, name: String) {
    let rect = NSRect(x: 160, y: 160, width: 720, height: 300)
    let backdrop = NSWindow(contentRect: rect, styleMask: .borderless, backing: .buffered, defer: false)
    let window = NSWindow(contentRect: rect, styleMask: .borderless, backing: .buffered, defer: false)
    backdrop.isReleasedWhenClosed = false
    window.isReleasedWhenClosed = false
    backdrop.level = .floating
    window.level = .floating
    window.isOpaque = false
    window.backgroundColor = .clear
    backdrop.ignoresMouseEvents = true
    window.ignoresMouseEvents = true
    defer { window.orderOut(nil); backdrop.orderOut(nil); window.close(); backdrop.close() }
    let root = window.contentView!
    let host = NSView()
    let shortcuts = NSView()
    for view in [host, shortcuts, glass] {
        view.translatesAutoresizingMaskIntoConstraints = false
        view.wantsLayer = true
    }
    root.addSubview(host)
    root.addSubview(shortcuts)
    host.addSubview(glass)
    let width = host.widthAnchor.constraint(equalToConstant: 160)
    NSLayoutConstraint.activate([
        host.centerXAnchor.constraint(equalTo: root.centerXAnchor),
        host.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -48),
        width, host.heightAnchor.constraint(equalToConstant: 40),
        glass.leadingAnchor.constraint(equalTo: host.leadingAnchor),
        glass.trailingAnchor.constraint(equalTo: host.trailingAnchor),
        glass.topAnchor.constraint(equalTo: host.topAnchor),
        glass.bottomAnchor.constraint(equalTo: host.bottomAnchor),
        shortcuts.centerXAnchor.constraint(equalTo: root.centerXAnchor),
        shortcuts.widthAnchor.constraint(equalTo: root.widthAnchor, constant: -64),
        shortcuts.heightAnchor.constraint(equalToConstant: 18),
        shortcuts.bottomAnchor.constraint(equalTo: host.topAnchor, constant: -16),
    ])
    let content = AdaptiveContent()
    if #available(macOS 26, *), let liquid = glass as? NSGlassEffectView {
        liquid.style = .regular
        liquid.cornerRadius = 20
        liquid.contentView = content
    } else if let vibrancy = glass as? NSVisualEffectView {
        vibrancy.state = .active
        vibrancy.material = .hudWindow
        vibrancy.blendingMode = .behindWindow
        vibrancy.addSubview(content)
    }
    let label = NSTextField(labelWithString: "准备中")
    label.alignment = .center
    content.addSubview(label)
    let keycap = NSView(frame: NSRect(x: 285, y: 0, width: 30, height: 18))
    keycap.wantsLayer = true
    keycap.layer!.cornerRadius = 4
    let key = NSTextField(labelWithString: "Fn")
    let action = NSTextField(labelWithString: "结束")
    key.frame = NSRect(x: 0, y: 0, width: 30, height: 18)
    key.alignment = .center
    action.frame = NSRect(x: 319, y: 0, width: 40, height: 18)
    keycap.addSubview(key)
    shortcuts.addSubview(keycap)
    shortcuts.addSubview(action)
    let icon = CAShapeLayer()
    content.wantsLayer = true
    content.layer!.addSublayer(icon)
    func syncForeground() {
        shortcuts.appearance = content.effectiveAppearance
        content.effectiveAppearance.performAsCurrentDrawingAppearance {
            key.textColor = .labelColor
            action.textColor = .labelColor
            keycap.layer!.backgroundColor = NSColor.labelColor.withAlphaComponent(0.12).cgColor
            icon.strokeColor = NSColor.labelColor.cgColor
        }
    }
    content.appearanceChanged = syncForeground
    backdrop.orderFrontRegardless()
    window.orderFrontRegardless()
    root.layoutSubtreeIfNeeded()
    syncForeground()
    let hintFrame = root.convert(shortcuts.bounds, from: shortcuts)
    let pillFrame = root.convert(host.bounds, from: host)
    let gap = root.isFlipped ? pillFrame.minY - hintFrame.maxY : hintFrame.minY - pillFrame.maxY
    precondition(abs(gap - 16) < 0.1, "shortcuts must sit 16 px above the pill, got \(gap)")
    for color in [NSColor.black, NSColor.white] {
        backdrop.backgroundColor = color
        advance(0.35)
        syncForeground()
        let source = content.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua])!
        precondition(shortcuts.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == source)
        var expected: CGColor!
        content.effectiveAppearance.performAsCurrentDrawingAppearance { expected = NSColor.labelColor.cgColor }
        precondition(icon.strokeColor == expected, "icon did not follow the transcript foreground")
        var shortcutColor: CGColor!
        shortcuts.effectiveAppearance.performAsCurrentDrawingAppearance { shortcutColor = key.textColor!.cgColor }
        precondition(shortcutColor == expected, "shortcut did not follow the transcript foreground")
        for next in [240.0, 400.0, 570.0] {
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0
                context.allowsImplicitAnimation = false
                width.constant = next
                root.layoutSubtreeIfNeeded()
            }
            for duration in [0.01, 0.03, 0.08] {
                advance(duration)
                precondition(abs(host.layer!.presentation()!.bounds.width - next) < 0.5, "pill width must update without animation")
                let presented = shortcuts.layer!.presentation()!
                let center = presented.convert(CGPoint(x: presented.bounds.midX, y: presented.bounds.midY), to: root.layer!.presentation()!)
                precondition(abs(center.x - root.bounds.midX) < 0.5, "shortcut drifted during width changes")
            }
            content.frame = glass.bounds
            label.frame = NSRect(x: 16, y: 10, width: content.bounds.width - 32, height: 20)
            precondition(abs(label.frame.midX - content.bounds.midX) < 0.1)
        }
        print("\(name): backdrop \(color == .black ? "black" : "white"), content appearance \(source.rawValue), fixed center and matching foreground passed")
    }
    // Changes while no transcript arrives must still update the outside hints.
    content.appearance = NSAppearance(named: .darkAqua)
    precondition(shortcuts.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua)
    content.appearance = NSAppearance(named: .aqua)
    precondition(shortcuts.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .aqua)
    precondition(keycap.layer!.cornerRadius == 4)
}

checkContent(NSVisualEffectView(), name: "Vibrancy")
if #available(macOS 26, *) { checkContent(NSGlassEffectView(), name: "Liquid Glass") }

// State labels must measure their full content even after an icon-only frame.
let status = NSTextField(labelWithString: "")
status.font = .systemFont(ofSize: 14)
for text in ["准备中", "识别中", "润色中", "准备中"] {
    status.frame = NSRect(x: 16, y: 0, width: 10, height: 20)
    status.stringValue = text
    status.usesSingleLineMode = true
    status.maximumNumberOfLines = 1
    status.preferredMaxLayoutWidth = 0
    status.lineBreakMode = .byClipping
    let measured = status.cell!.cellSize
    status.lineBreakMode = .byTruncatingTail
    let textWidth = ceil(measured.width) + 2
    status.frame.size.width = textWidth
    let required = (text as NSString).size(withAttributes: [.font: status.font!]).width
    let drawingWidth = status.cell!.drawingRect(forBounds: status.bounds).width
    precondition(drawingWidth >= required, "\(text) is truncated: available \(drawingWidth), required \(required)")
}
print("State labels: full text fits after narrow frames")

let fadeView = NSView(frame: NSRect(x: 0, y: 0, width: 300, height: 18))
fadeView.wantsLayer = true
fadeView.alphaValue = 0
let fadeWindow = NSWindow(contentRect: NSRect(x: 160, y: 160, width: 320, height: 80), styleMask: .borderless, backing: .buffered, defer: false)
fadeWindow.isReleasedWhenClosed = false
fadeWindow.ignoresMouseEvents = true
fadeWindow.contentView!.wantsLayer = true
fadeWindow.contentView!.addSubview(fadeView)
fadeView.addSubview(NSTextField(labelWithString: "Fn 结束"))
fadeWindow.orderFrontRegardless()
advance(0.1)
func fade(_ visible: Bool) {
    let layer = fadeView.layer!
    let from = layer.presentation()?.opacity ?? layer.opacity
    CATransaction.begin()
    CATransaction.setDisableActions(true)
    fadeView.alphaValue = visible ? 1 : 0
    CATransaction.commit()
    let animation = CABasicAnimation(keyPath: "opacity")
    animation.fromValue = from
    animation.toValue = visible ? 1.0 : 0.0
    animation.duration = visible ? 0.16 : 0.12
    animation.timingFunction = CAMediaTimingFunction(name: visible ? .easeOut : .easeIn)
    layer.add(animation, forKey: "vp-shortcuts-opacity")
}
fade(true)
precondition(fadeView.alphaValue == 1 && fadeView.layer!.animation(forKey: "vp-shortcuts-opacity")!.duration == 0.16)
advance(0.05)
let entering = fadeView.layer!.presentation()!.opacity
precondition(entering > 0 && entering < 1, "entry must fade progressively")
fade(false)
precondition(fadeView.alphaValue == 0 && fadeView.layer!.animation(forKey: "vp-shortcuts-opacity")!.duration == 0.12)
advance(0.03)
let exiting = fadeView.layer!.presentation()!.opacity
fade(true)
let resumed = (fadeView.layer!.animation(forKey: "vp-shortcuts-opacity") as! CABasicAnimation).fromValue as! NSNumber
precondition(abs(resumed.floatValue - exiting) < 0.01, "interrupted exit must resume without a jump")
advance(0.2)
precondition(fadeView.layer!.presentation()!.opacity == 1)
fade(false)
advance(0.2)
precondition(fadeView.layer!.presentation()!.opacity == 0)
fadeWindow.orderOut(nil)
fadeWindow.close()
print("Shortcut fade: entry, exit and interrupted exit passed")
