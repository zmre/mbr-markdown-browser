import Foundation
import Observation

/// One hit from `POST /.mbr/search` (`mbr_core::search::SearchResult`).
struct SearchHit: Decodable, Identifiable, Hashable {
    /// Site-absolute URL path, percent-*decoded* (`/notes/a b/`).
    let urlPath: String
    let title: String?
    let description: String?
    let snippet: String?
    let isContentMatch: Bool
    let filetype: String

    var id: String {
        self.urlPath
    }

    /// The title, else the page's last path segment.
    var displayTitle: String {
        if let title = self.title?.trimmingCharacters(in: .whitespacesAndNewlines), !title.isEmpty {
            return title
        }
        return self.urlPath.split(separator: "/").last.map(String.init) ?? self.urlPath
    }

    /// The folder the page is in (`/notes/` for `/notes/idea/`), `/` at the top.
    var folder: String {
        var parts = self.urlPath.split(separator: "/")
        if !parts.isEmpty { parts.removeLast() }
        return "/" + parts.map { "\($0)/" }.joined()
    }

    /// `urlPath` encoded for a URL: Rust stores it decoded, so a space
    /// would otherwise make `URL(string:)` fail.
    var encodedPath: String {
        self.urlPath.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? self.urlPath
    }
}

/// The body of a search answer (`mbr_core::search::SearchResponse`), or the
/// server's `{error}` on a failure.
private struct SearchAnswer: Decodable {
    let totalMatches: Int?
    let results: [SearchHit]?
    let durationMs: Int?
    let scanInProgress: Bool?
    let notDownloaded: Int?
    let error: String?
}

/// What the last finished search found.
struct SearchOutcome: Equatable {
    let query: String
    let hits: [SearchHit]
    let totalMatches: Int
    let durationMs: Int
    /// Notes content search skipped because their bytes are not on the device.
    let notDownloaded: Int
    /// The vault's first scan was still running: results may be incomplete.
    let scanInProgress: Bool
}

/// The native search sheet's model: queries the vault's in-process server the
/// way `<mbr-search>` does, with the web modal's defaults (all scopes, the
/// whole vault, markdown only, 20 results).
@MainActor
@Observable
final class VaultSearch {
    /// Queries shorter than this are not sent (the web modal's rule).
    static let minimumQueryLength = 2
    static let limit = 20
    /// Typing pause before a query goes out.
    static let debounce: Duration = .milliseconds(200)

    private(set) var outcome: SearchOutcome?
    private(set) var error: String?
    private(set) var isSearching = false

    private let app: MbrApp

    init(app: MbrApp) {
        self.app = app
    }

    /// Debounces, then runs `query`. Meant for `.task(id: query)`: SwiftUI
    /// cancels the previous call when the query changes, and a cancelled call
    /// changes nothing, so a slow answer can never overwrite a newer one.
    func run(_ query: String) async {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard query.count >= Self.minimumQueryLength else {
            self.outcome = nil
            self.error = nil
            self.isSearching = false
            return
        }
        do {
            try await Task.sleep(for: Self.debounce)
        } catch {
            return
        }
        self.isSearching = true
        let started = ContinuousClock.now
        let result: Result<SearchOutcome, Error>
        do {
            result = try await .success(self.search(query))
        } catch {
            result = .failure(error)
        }
        // Superseded while Rust was answering: the newer call owns the state.
        guard !Task.isCancelled else { return }
        self.isSearching = false
        switch result {
        case let .success(outcome):
            self.outcome = outcome
            self.error = nil
            Log.info("search: \"\(query)\" -> \(outcome.hits.count)/\(outcome.totalMatches) "
                + "in \(outcome.durationMs)ms (\(millis(since: started))ms round trip)"
                + (outcome.notDownloaded > 0 ? ", \(outcome.notDownloaded) not downloaded" : ""))
        case let .failure(error):
            self.outcome = nil
            self.error = (error as? SearchFailure)?.message ?? error.localizedDescription
            Log.info("search: \"\(query)\" failed: \(error)")
        }
    }

    /// One `POST /.mbr/search` through the in-process router.
    private func search(_ query: String) async throws -> SearchOutcome {
        // Field names are the Rust `SearchQuery`'s; it ignores unknown keys,
        // so a typo here would silently fall back to a default.
        let request: [String: Any] = [
            "q": query,
            "limit": Self.limit,
            "scope": "all",
            "folder_scope": "everywhere"
        ]
        let body = try JSONSerialization.data(withJSONObject: request)
        let response = try await self.app.handle(
            method: "POST",
            url: MbrSchemeHandler.url(forPath: "/.mbr/search").absoluteString,
            headers: [
                MobileHeader(name: "Content-Type", value: "application/json"),
                MobileHeader(name: "Accept", value: "application/json")
            ],
            body: body
        )
        let status = Int(response.status())
        let data = try await response.readAll()
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let answer = try? decoder.decode(SearchAnswer.self, from: data)
        if let message = answer?.error { throw SearchFailure(message: message) }
        guard (200 ..< 300).contains(status), let answer, let results = answer.results else {
            throw SearchFailure(message: "Search failed (HTTP \(status))")
        }
        // One row per page: a page can match on metadata and content.
        var seen = Set<String>()
        let hits = results.filter { seen.insert($0.urlPath).inserted }
        return SearchOutcome(
            query: query,
            hits: hits,
            totalMatches: answer.totalMatches ?? hits.count,
            durationMs: answer.durationMs ?? 0,
            notDownloaded: answer.notDownloaded ?? 0,
            scanInProgress: answer.scanInProgress ?? false
        )
    }
}

/// A search the server answered with an error, or not as JSON.
struct SearchFailure: Error, CustomStringConvertible {
    let message: String
    var description: String {
        self.message
    }
}
