# Installs a prebuilt app; requires a signed, notarized release artifact.
cask "continuum" do
  version "0.1.4"
  sha256 :no_check

  url "https://github.com/JonRoosevelt/continuum/releases/download/v#{version}/Continuum-#{version}.zip"
  name "Continuum"
  desc "Clipboard sync between macOS and Linux over the LAN"
  homepage "https://github.com/JonRoosevelt/continuum"

  app "Continuum.app"

  uninstall launchctl: "dev.continuum.agent"
  zap trash: ["~/Library/Application Support/continuum"]
end
