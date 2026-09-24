# Template. Replace OWNER and publish a signed, notarized .zip for real use.
cask "continuum" do
  version "0.1.0"
  sha256 :no_check

  url "https://github.com/OWNER/continuum/releases/download/v#{version}/Continuum-#{version}.zip"
  name "Continuum"
  desc "Clipboard sync between macOS and Linux over the LAN"
  homepage "https://github.com/OWNER/continuum"

  app "Continuum.app"

  uninstall launchctl: "dev.continuum.agent"
  zap trash: ["~/Library/Application Support/continuum"]
end
