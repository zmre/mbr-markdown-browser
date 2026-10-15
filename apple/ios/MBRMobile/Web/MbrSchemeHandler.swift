import Foundation
import WebKit

/// What a page's scheme handler serves from. The web page is created (and its
/// WebContent process spawned) before the vault has opened, so the handler
/// holds this box rather than the `MbrApp` itself; the session fills it in.
final class SchemeTarget: @unchecked Sendable {
    private let lock = NSLock()
    private var currentApp: MbrApp?
    private var documentRequests: [String: ContinuousClock.Instant] = [:]
    private var retryHandler: (@Sendable (URL, Int) -> Void)?

    var app: MbrApp? {
        get { self.lock.withLock { self.currentApp } }
        set { self.lock.withLock { self.currentApp = newValue } }
    }

    /// Called when a page load is answered `503` with `Retry-After` (a note
    /// that is not downloaded yet), with the URL and the delay in seconds.
    func onRetryableDocument(_ handler: @escaping @Sendable (URL, Int) -> Void) {
        self.lock.withLock { self.retryHandler = handler }
    }

    fileprivate func noteDocumentRequest(_ url: URL) {
        self.lock.withLock { self.documentRequests[url.absoluteString] = .now }
    }

    /// When `url` was last requested as a page load.
    func lastDocumentRequest(_ url: URL) -> ContinuousClock.Instant? {
        self.lock.withLock { self.documentRequests[url.absoluteString] }
    }

    fileprivate func retryableDocument(_ url: URL, after seconds: Int) {
        let handler = self.lock.withLock { self.retryHandler }
        handler?(url, seconds)
    }
}

/// Serves `mbr://localhost/...` by handing each request to the in-process mbr
/// router (`MbrApp.handle`): no socket, no port.
///
/// Pages must be loaded from `mbr://localhost/`: Rust derives the `Host`
/// header from the URL authority, and mbr's edit gate requires `localhost`.
struct MbrSchemeHandler: URLSchemeHandler {
    static let scheme = "mbr"
    static let origin = URL(string: "mbr://localhost/")!

    let target: SchemeTarget

    func reply(for request: URLRequest) -> MbrReply {
        MbrReply(request: request, target: target)
    }

    /// `mbr://localhost` plus a URL path from Rust (`/notes/idea/`).
    static func url(forPath path: String) -> URL {
        URL(string: path, relativeTo: self.origin)?.absoluteURL ?? self.origin
    }
}

/// One response, pulled a chunk at a time: WebKit asks for the next piece only
/// when it wants it, and each piece is one `MbrResponse.nextChunk()`, so a
/// video streams with Rust's small bounded buffer and never sits in memory.
/// Iterated off the main actor (a nonisolated async iterator); the Rust calls
/// run on the vault's own runtime.
struct MbrReply: AsyncSequence {
    typealias Element = URLSchemeTaskResult
    typealias Failure = any Error

    let request: URLRequest
    let target: SchemeTarget

    func makeAsyncIterator() -> MbrReplyIterator {
        MbrReplyIterator(request: self.request, target: self.target)
    }
}

/// `MbrReply`'s iterator: the response head first, then one chunk per call.
struct MbrReplyIterator: AsyncIteratorProtocol {
    typealias Element = URLSchemeTaskResult
    typealias Failure = any Error

    let request: URLRequest
    let target: SchemeTarget
    private var body: MbrResponse?
    private var done = false
    private var bytes = 0
    private var summary = ""
    private let started = ContinuousClock.now

    init(request: URLRequest, target: SchemeTarget) {
        self.request = request
        self.target = target
    }

    mutating func next() async throws -> URLSchemeTaskResult? {
        if self.done { return nil }
        guard let body else {
            do {
                let (response, head) = try await start()
                self.body = response
                return .response(head)
            } catch {
                self.done = true
                Log.info("scheme: \(self.request.url?.absoluteString ?? "?") failed: \(error)")
                throw error
            }
        }
        do {
            if let chunk = try await body.nextChunk() {
                self.bytes += chunk.count
                return .data(chunk)
            }
        } catch {
            self.done = true
            Log.info("\(self.summary) body broke off after \(self.bytes)B: \(error)")
            throw error
        }
        self.done = true
        Log.info("\(self.summary) \(self.bytes)B in \(millis(since: self.started))ms")
        return nil
    }

    private mutating func start() async throws -> (MbrResponse, URLResponse) {
        guard let url = request.url else { throw URLError(.badURL) }
        guard let app = target.app else { throw URLError(.cannotConnectToHost) }
        let method = self.request.httpMethod ?? "GET"
        let accept = self.request.value(forHTTPHeaderField: "Accept") ?? ""
        let isDocument = method == "GET" && accept.contains("text/html")
        if isDocument { self.target.noteDocumentRequest(url) }

        let (requestBody, bodySource) = Self.body(of: self.request)
        let headers = (request.allHTTPHeaderFields ?? [:]).map { MobileHeader(name: $0.key, value: $0.value) }
        let response = try await app.handle(
            method: method,
            url: url.absoluteString,
            headers: headers,
            body: requestBody
        )
        let status = Int(response.status())
        let fields = Self.fields(response.headers())

        self.summary = "\(method) \(url.path(percentEncoded: true)) -> \(status) \(fields["Content-Type"] ?? "-")"
        if let range = request.value(forHTTPHeaderField: "Range") {
            self.summary += " [Range \(range) -> \(fields["Content-Range"] ?? "none")]"
        }
        if method != "GET", method != "HEAD" {
            self.summary += " [request body: \(bodySource) \(requestBody.count)B]"
        }
        if isDocument, status == 503, let retry = fields["Retry-After"].flatMap(Int.init) {
            self.summary += " [Retry-After \(retry)s, Refresh \(fields["Refresh"] ?? "-")]"
            self.target.retryableDocument(url, after: retry)
        }

        guard let head = HTTPURLResponse(
            url: url,
            statusCode: status,
            httpVersion: "HTTP/1.1",
            headerFields: fields
        )
        else { throw URLError(.cannotParseResponse) }
        return (response, head)
    }

    /// The request body, wherever WebKit put it. JSON string bodies (all
    /// mbr's v1 endpoints send) arrive in `httpBody`; `Blob` bodies arrive
    /// in neither (a known WebKit limitation, research doc §12.3).
    private static func body(of request: URLRequest) -> (Data, String) {
        if let data = request.httpBody { return (data, "httpBody") }
        guard let stream = request.httpBodyStream else { return (Data(), "none") }
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 64 * 1024)
        while stream.hasBytesAvailable {
            let count = stream.read(&buffer, maxLength: buffer.count)
            if count <= 0 { break }
            data.append(buffer, count: count)
        }
        return (data, "httpBodyStream")
    }

    /// Header list to a dictionary, keyed by canonical names so lookups
    /// work. Repeated headers fold with ", ", which is right for
    /// everything mbr sends.
    private static func fields(_ headers: [MobileHeader]) -> [String: String] {
        Dictionary(headers.map { (self.canonical($0.name), $0.value) }, uniquingKeysWith: { "\($0), \($1)" })
    }

    private static func canonical(_ name: String) -> String {
        name.split(separator: "-")
            .map { $0.prefix(1).uppercased() + $0.dropFirst().lowercased() }
            .joined(separator: "-")
    }
}
