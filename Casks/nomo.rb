cask "nomo" do
  version "0.5.3"
  sha256 "5dbc3c40425eabfb2625f40a1482a1b263f6f3cfa0de0ac7d5d6750ec6c9aa5a"

  url "https://github.com/nomo-md/nomo/releases/download/v#{version}/Nomo_#{version}_aarch64.dmg"
  name "Nomo"
  desc "Local-first Markdown desktop editor"
  homepage "https://github.com/nomo-md/nomo"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: :monterey

  app "Nomo.app"

  zap trash: [
    "~/Library/Application Support/com.nomo.desktop",
    "~/Library/Caches/com.nomo.desktop",
    "~/Library/Logs/com.nomo.desktop",
    "~/Library/Preferences/com.nomo.desktop.plist",
    "~/Library/WebKit/com.nomo.desktop",
  ]
end
