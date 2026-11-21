// Host-only observation of actual simulator screenshots; never linked into the app.
import Foundation
import Vision

guard CommandLine.arguments.count == 2 else {
    fatalError("Usage: ios-client-ocr screenshot.png")
}
let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
request.recognitionLanguages = ["en-US"]
request.usesLanguageCorrection = false
let handler = VNImageRequestHandler(url: URL(fileURLWithPath: CommandLine.arguments[1]))
try handler.perform([request])
let lines = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
let data = try JSONSerialization.data(withJSONObject: lines, options: [.sortedKeys])
FileHandle.standardOutput.write(data)
