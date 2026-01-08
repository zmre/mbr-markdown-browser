// MBRQuickLookHostApp.swift
// Minimal host app for the QuickLook extension.
// This app is required by Apple for embedding app extensions but doesn't
// need any functionality - users interact with the extension through Finder.

import SwiftUI

@main
struct MBRQuickLookHostApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

struct ContentView: View {
    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: "doc.text.magnifyingglass")
                .resizable()
                .scaledToFit()
                .frame(width: 80, height: 80)
                .foregroundColor(.secondary)

            Text("MBR Markdown Preview")
                .font(.title)

            Text("This app provides QuickLook preview support for Markdown files.")
                .multilineTextAlignment(.center)
                .foregroundColor(.secondary)
                .padding(.horizontal)

            Divider()
                .frame(width: 200)

            VStack(alignment: .leading, spacing: 8) {
                Text("How to use:")
                    .font(.headline)

                Text("1. Select a .md file in Finder")
                Text("2. Press Space to preview")
                Text("3. Enjoy beautiful markdown rendering!")
            }
            .font(.callout)
            .foregroundColor(.secondary)

            Spacer()

            Link("Learn more at github.com/zmre/mbr",
                 destination: URL(string: "https://github.com/zmre/mbr")!)
                .font(.caption)
        }
        .padding(40)
        .frame(width: 400, height: 350)
    }
}
