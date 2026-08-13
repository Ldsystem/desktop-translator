// Renders the macOS menu-bar template icon.
//
// Template images must contain only black pixels plus alpha; macOS tints them
// for light, dark, and highlighted menu bars. Run from the repository root:
//
//   swift tools/icons/render-tray-template.swift src-tauri/icons/tray-template.png

import AppKit

let canvas = NSSize(width: 44, height: 44)
let glyphPointSize: CGFloat = 34
let candidateSymbols = ["translate", "character.bubble", "globe"]

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: render-tray-template.swift <output.png>\n".utf8))
    exit(2)
}
let outputPath = CommandLine.arguments[1]

let configuration = NSImage.SymbolConfiguration(pointSize: glyphPointSize, weight: .regular)
guard
    let symbol = candidateSymbols
        .lazy
        .compactMap({ NSImage(systemSymbolName: $0, accessibilityDescription: "Translate") })
        .first?
        .withSymbolConfiguration(configuration)
else {
    FileHandle.standardError.write(Data("no candidate SF Symbol is available\n".utf8))
    exit(1)
}

guard
    let representation = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: Int(canvas.width),
        pixelsHigh: Int(canvas.height),
        bitsPerSample: 8,
        samplesPerPixel: 4,
        hasAlpha: true,
        isPlanar: false,
        colorSpaceName: .deviceRGB,
        bytesPerRow: 0,
        bitsPerPixel: 0
    )
else {
    FileHandle.standardError.write(Data("bitmap allocation failed\n".utf8))
    exit(1)
}

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: representation)
NSColor.clear.setFill()
NSRect(origin: .zero, size: canvas).fill()

// Fit the glyph inside the canvas, then stencil it in pure black so the image
// qualifies as a template.
let glyphSize = symbol.size
let scale = min(canvas.width / glyphSize.width, canvas.height / glyphSize.height)
let drawnSize = NSSize(width: glyphSize.width * scale, height: glyphSize.height * scale)
let drawnRect = NSRect(
    x: (canvas.width - drawnSize.width) / 2,
    y: (canvas.height - drawnSize.height) / 2,
    width: drawnSize.width,
    height: drawnSize.height
)
symbol.draw(in: drawnRect)
NSColor.black.setFill()
drawnRect.fill(using: .sourceAtop)
NSGraphicsContext.restoreGraphicsState()

guard let png = representation.representation(using: .png, properties: [:]) else {
    FileHandle.standardError.write(Data("png encoding failed\n".utf8))
    exit(1)
}
try png.write(to: URL(fileURLWithPath: outputPath))
