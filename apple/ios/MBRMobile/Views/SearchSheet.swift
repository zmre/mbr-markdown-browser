import SwiftUI

/// ⋯ → Search: a native search over the vault, presented as a sheet.
///
/// Native rather than the page's `<mbr-search>` modal because WebKit raises the
/// software keyboard for a script `focus()` only while the user is touching the
/// web view, and a choice in a native menu is not that: the web modal opened
/// from ⋯ had a focused input and no keyboard. A native field gets one at once.
struct SearchSheet: View {
    let session: VaultSession
    @Environment(\.dismiss) private var dismiss
    @State private var search: VaultSearch
    @State private var query = ""
    @State private var fieldPresented = false

    init(session: VaultSession) {
        self.session = session
        self._search = State(initialValue: VaultSearch(app: session.app))
    }

    var body: some View {
        NavigationStack {
            self.content
                .navigationTitle("Search")
                .navigationBarTitleDisplayMode(.inline)
                .searchable(
                    text: self.$query,
                    isPresented: self.$fieldPresented,
                    placement: .navigationBarDrawer(displayMode: .always),
                    prompt: "Titles, tags and text"
                )
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                .onSubmit(of: .search) { self.openFirst() }
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Close", systemImage: "xmark") { self.dismiss() }
                            .accessibilityIdentifier("searchClose")
                    }
                }
                // SwiftUI cancels the previous run on every keystroke, which is
                // both the debounce and the stale-answer guard.
                .task(id: self.query) { await self.search.run(self.query) }
                // Activating the field as the sheet appears is what brings the
                // keyboard up with it.
                .onAppear { self.fieldPresented = true }
                // While the field is active iOS replaces the bar (and Close)
                // with the field and its own X, which only deactivates it. The
                // sheet is nothing but the search, so that X closes it too.
                .onChange(of: self.fieldPresented) { wasPresented, isPresented in
                    if wasPresented, !isPresented { self.dismiss() }
                }
        }
    }

    @ViewBuilder
    private var content: some View {
        let trimmed = self.query.trimmingCharacters(in: .whitespacesAndNewlines)
        if let error = self.search.error {
            ContentUnavailableView {
                Label("Search failed", systemImage: "exclamationmark.triangle")
            } description: {
                Text(error)
            }
        } else if let outcome = self.search.outcome, trimmed.count >= VaultSearch.minimumQueryLength {
            if outcome.hits.isEmpty, !self.search.isSearching {
                ContentUnavailableView {
                    Label("No Results", systemImage: "magnifyingglass")
                } description: {
                    Text("Nothing in this vault matches “\(outcome.query)”.")
                }
                .safeAreaInset(edge: .bottom) { self.footnotes(outcome) }
            } else {
                self.results(outcome)
            }
        } else if trimmed.count >= VaultSearch.minimumQueryLength {
            ProgressView("Searching…")
        } else {
            ContentUnavailableView(
                "Search \(self.session.title)",
                systemImage: "magnifyingglass",
                description: Text("Matches titles, tags and note text. Use key:value for frontmatter, e.g. tags:ideas.")
            )
        }
    }

    private func results(_ outcome: SearchOutcome) -> some View {
        List {
            Section {
                ForEach(outcome.hits) { hit in
                    Button {
                        self.open(hit)
                    } label: {
                        SearchHitRow(hit: hit)
                    }
                    .accessibilityIdentifier("searchResult")
                }
            } footer: {
                self.footnotes(outcome)
            }
        }
        .listStyle(.plain)
        .opacity(self.search.isSearching ? 0.6 : 1)
    }

    private func footnotes(_ outcome: SearchOutcome) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            if outcome.totalMatches > outcome.hits.count {
                Text("Showing \(outcome.hits.count) of \(outcome.totalMatches) matches.")
            }
            if outcome.notDownloaded > 0 {
                Label(
                    outcome.notDownloaded == 1
                        ? "1 note is not downloaded and was not searched."
                        : "\(outcome.notDownloaded) notes are not downloaded and were not searched.",
                    systemImage: "icloud.and.arrow.down"
                )
            }
            if outcome.scanInProgress {
                Label("Still indexing: results may be incomplete.", systemImage: "hourglass")
            }
        }
        .font(.footnote)
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal)
    }

    private func openFirst() {
        if let first = self.search.outcome?.hits.first { self.open(first) }
    }

    private func open(_ hit: SearchHit) {
        Log.info("search: open \(hit.urlPath)")
        self.dismiss()
        let session = self.session
        Task { await session.load(MbrSchemeHandler.url(forPath: hit.encodedPath), label: "search") }
    }
}

/// One result: title, folder, and the matched text when there is some.
private struct SearchHitRow: View {
    let hit: SearchHit

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline) {
                Text(self.hit.displayTitle)
                    .font(.body.weight(.semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(2)
                Spacer(minLength: 8)
                if self.hit.filetype != "markdown" {
                    Text(self.hit.filetype)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            Text(self.hit.folder)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.head)
            if let detail = self.detail {
                Text(detail)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
            }
        }
        .padding(.vertical, 2)
        .contentShape(.rect)
    }

    /// The content snippet, else the frontmatter description.
    private var detail: String? {
        let text = (self.hit.snippet.map(Self.withoutLineNumber) ?? self.hit.description)?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return text?.isEmpty == false ? text : nil
    }

    /// Content snippets start `Line 12: `, which means nothing on a phone.
    private static func withoutLineNumber(_ snippet: String) -> String {
        guard let match = snippet.firstMatch(of: #/^Line \d+: /#) else { return snippet }
        return String(snippet[match.range.upperBound...])
    }
}
