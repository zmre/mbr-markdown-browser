cask "mbr" do
  version "0.6.2"
  sha256 "041f2b5e5d4f99fb8e91185f539648b0641c0481fba927e8020afc8f30c4909b"

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
