#!/usr/bin/env ruby
# frozen_string_literal: true

# Generate exactly 1,000 hard safety rules for irreplaceable user/application
# data. Files start with `_protected-`, so cleanup RulePack ignores them.

require "fileutils"
require "yaml"

ROOT = File.expand_path("..", __dir__)
OUT = File.join(ROOT, "rules", "protected")

APPS = %w[
  1password bitwarden keepassxc lastpass authy microsoft-authenticator
  google-authenticator proton-pass dashlane nordpass keeper enpass yubico
  electrum exodus metamask ledger-live trezor-suite phantom solflare
  sparrow-wallet wasabi-wallet monero-wallet bitcoin-core ethereum
  chrome chromium firefox safari edge brave vivaldi opera tor-browser arc
  thunderbird outlook proton-mail mailbird spark-mail airmail mailspring
  signal telegram whatsapp element discord slack teams zoom skype
  messages facetime contacts calendars reminders notes notion obsidian
  evernote onenote bear craft todoist things omnifocus logseq roam
  vscode cursor zed jetbrains xcode android-studio visual-studio
  github-desktop sourcetree tower fork gitkraken docker-desktop
  dropbox onedrive google-drive icloud-drive nextcloud syncthing box-drive
  photos lightroom photoshop affinity-photo capture-one davinci-resolve
  final-cut-pro logic-pro ableton-live pro-tools premiere-pro blender
].freeze

abort "expected 100 protected app families, got #{APPS.length}" unless APPS.length == 100

PLATFORMS = {
  "macos" => {
    root: "$HOME/Library/Application Support",
    suffixes: ["User Data", "Profiles", "Databases", "Local Storage"]
  },
  "windows" => {
    root: "%APPDATA%",
    suffixes: ["User Data", "Profiles", "Databases"]
  },
  "linux" => {
    root: "$HOME/.config",
    suffixes: ["User Data", "Profiles", "databases"]
  }
}.freeze

def slug(value)
  value.downcase.gsub(/[^a-z0-9]+/, "-").gsub(/-+/, "-").sub(/^-|-$/, "")
end

FileUtils.mkdir_p(OUT)
total = 0

PLATFORMS.each do |platform, spec|
  protections = []
  APPS.each do |app|
    spec[:suffixes].each do |suffix|
      protections << {
        "id" => "protect-#{platform}-#{slug(app)}-#{slug(suffix)}",
        "name" => "Protect #{app} #{suffix}",
        "description" => "Hard safety boundary for credentials, profiles, settings, or user-created data.",
        "platform" => platform,
        "paths" => ["#{spec[:root]}/#{app}/#{suffix}"],
        "tags" => ["never-delete", slug(app), slug(suffix)]
      }
    end
  end

  path = File.join(OUT, "_protected-#{platform}.yaml")
  File.write(path, YAML.dump({
    "pack" => "protected-#{platform}",
    "description" => "Generated hard deletion guards; do not load as cleanup rules.",
    "protections" => protections
  }))
  total += protections.length
  puts "#{path}: #{protections.length}"
end

abort "expected exactly 1000 protections, got #{total}" unless total == 1_000
puts "total: #{total}"
