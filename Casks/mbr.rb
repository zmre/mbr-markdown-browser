cask "mbr" do
  version "0.7.2"
  sha256 "1816724409125b677a19329d3cc28837b1e8764652ec3f2fff1b04a5250fa584"

  url "https://github.com/zmre/mbr-markdown-browser/releases/download/v#{version}/mbr-macos-arm64.dmg"
  name "MBR"
  desc "Markdown browser, previewer and static site generator"
  homepage "https://github.com/zmre/mbr-markdown-browser"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "MBR.app"
  binary "#{appdir}/MBR.app/Contents/MacOS/mbr"

  zap trash: [
    "~/Library/Application Scripts/com.zmre.mbr.MBRPreview",
    "~/Library/Caches/com.zmre.mbr",
    "~/Library/Containers/com.zmre.mbr.MBRPreview",
    "~/Library/HTTPStorages/com.zmre.mbr",
    "~/Library/Preferences/com.zmre.mbr.plist",
    "~/Library/Saved Application State/com.zmre.mbr.savedState",
    "~/Library/WebKit/com.zmre.mbr",
  ]
end
