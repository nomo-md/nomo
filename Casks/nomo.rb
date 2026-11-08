cask "nomo" do
  version "0.7.0"
  sha256 "02b4a31bf39c0494de5bbb039a8f4d2f482d3992405f6b5c0f6bac5948243cab"

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
