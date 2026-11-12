cask "nomo" do
  version "0.7.2"
  sha256 "a3979f95bd2f290218325e8facbb597f3ce716ff0b58150ac0b5c220fd3a5db5"

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
