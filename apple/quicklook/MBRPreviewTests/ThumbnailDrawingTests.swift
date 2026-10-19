// ThumbnailDrawingTests.swift
// MBR Markdown QuickLook Extension Tests
//
// The thumbnail reply's drawing block must fill the context QuickLook hands
// it, whatever that context's scale and transform. In Finder it is a bitmap of
// contextSize x scale pixels with no scale CTM, and drawing into a
// contextSize rect filled only the bottom-left quarter at 2x.

import CoreGraphics
import XCTest

final class ThumbnailDrawingTests: XCTestCase {
    /// A bitmap context of `points` x `scale` pixels, as QuickLook hands the
    /// reply's drawing block, optionally with the scale already in its CTM.
    private func thumbnailContext(points: CGSize, scale: CGFloat, scaledCTM: Bool) throws -> CGContext {
        let context = try XCTUnwrap(CGContext(
            data: nil,
            width: Int(points.width * scale),
            height: Int(points.height * scale),
            bitsPerComponent: 8,
            bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ))
        if scaledCTM {
            context.scaleBy(x: scale, y: scale)
        }
        return context
    }

    /// The RGBA of the pixel at (`x`, `y`) of a bitmap context.
    private func pixel(_ context: CGContext, x: Int, y: Int) throws -> (red: UInt8, alpha: UInt8) {
        let data = try XCTUnwrap(context.data).assumingMemoryBound(to: UInt8.self)
        let offset = y * context.bytesPerRow + x * 4
        return (data[offset], data[offset + 3])
    }

    private func solidRedImage() throws -> CGImage {
        let context = try thumbnailContext(points: CGSize(width: 8, height: 8), scale: 1, scaledCTM: false)
        context.setFillColor(red: 1, green: 0, blue: 0, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: 8, height: 8))
        return try XCTUnwrap(context.makeImage())
    }

    func testDrawingRect_fillsWholeContextAtEveryScale() throws {
        let points = CGSize(width: 99, height: 128)
        let image = try solidRedImage()
        for scale in [1.0, 2.0, 3.0] as [CGFloat] {
            for scaledCTM in [false, true] {
                let context = try thumbnailContext(points: points, scale: scale, scaledCTM: scaledCTM)
                context.draw(image, in: ThumbnailProvider.drawingRect(in: context, fallback: points))

                let maxX = context.width - 1
                let maxY = context.height - 1
                for (x, y) in [(0, 0), (maxX, 0), (0, maxY), (maxX, maxY)] {
                    let painted = try pixel(context, x: x, y: y)
                    XCTAssertEqual(
                        painted.alpha,
                        255,
                        "corner (\(x), \(y)) unpainted at scale \(scale), scaled CTM \(scaledCTM)"
                    )
                    XCTAssertEqual(painted.red, 255)
                }
            }
        }
    }

    func testDrawingRect_isPointsUnderAScaledCTMAndPixelsWithout() throws {
        let points = CGSize(width: 50, height: 64)
        let plain = try thumbnailContext(points: points, scale: 2, scaledCTM: false)
        XCTAssertEqual(ThumbnailProvider.drawingRect(in: plain, fallback: points).size, CGSize(width: 100, height: 128))

        let scaled = try thumbnailContext(points: points, scale: 2, scaledCTM: true)
        XCTAssertEqual(ThumbnailProvider.drawingRect(in: scaled, fallback: points).size, points)
    }
}
