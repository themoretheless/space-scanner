#!/usr/bin/env ruby
# frozen_string_literal: true

# Import Winapp2 cookie + history FileKeys as opt-in privacy rules (settings toggles).
# Passwords / wallets / credentials stay excluded.

require "yaml"
require "set"
require "fileutils"

ROOT = File.expand_path("..", __dir__)
OUT = File.join(ROOT, "rules", "categories", "browser-privacy.yaml")
WINAPP = ENV.fetch("WINAPP2", "/tmp/Winapp2.ini")

COOKIE = /cookie/i
HISTORY = /history|places\.sqlite|typed url/i
SKIP = /(password|passwd|wallet|credential|login data|autofill|bookmark|form history|sync data)/i

def slug(s)
  s.to_s.downcase.gsub(/[^a-z0-9]+/, "-").gsub(/-+/, "-").sub(/^-|-$/, "")[0, 72]
end

def normalize_win_path(p)
  s = p.tr("\\", "/")
  s = s.gsub("%LocalAppData%", "%LOCALAPPDATA%")
  s = s.gsub("%AppData%", "%APPDATA%")
  s = s.gsub("%CommonAppData%", "%PROGRAMDATA%")
  s = s.gsub("%ProgramData%", "%PROGRAMDATA%")
  s = s.gsub("%UserProfile%", "%USERPROFILE%")
  s = s.gsub("%WinDir%", "%WINDIR%")
  s = s.gsub("%LocalLowAppData%", "%USERPROFILE%/AppData/LocalLow")
  s = s.gsub("%Documents%", "%USERPROFILE%/Documents")
  s = s.gsub("%ProgramFiles(x86)%", "%PROGRAMFILES(X86)%")
  s = s.gsub("%ProgramFiles%", "%PROGRAMFILES%")
  s = s.gsub("%SystemDrive%", "%SystemDrive%")
  s = s.gsub("%Temp%", "%TEMP%")
  s
end

# FileKey folder|pattern → [path, include_globs]
def parse_filekey(fk)
  folder, pattern, *_rest = fk.split("|")
  folder = folder.to_s
  pattern = pattern.to_s
  pattern = "" if %w[* RECURSE REMOVESELF].include?(pattern)
  glob = []
  if folder.include?("*")
    idx = folder.index("*")
    prefix = folder[0...idx].sub(/[\\\/]+$/, "")
    rest = folder[idx..].tr("\\", "/")
    glob << rest
    glob << "#{rest.sub(%r{/$}, '')}/#{pattern}" unless pattern.empty?
    glob << "**/" + pattern unless pattern.empty?
    [normalize_win_path(prefix), glob.uniq]
  else
    glob << pattern unless pattern.empty?
    [normalize_win_path(folder), glob]
  end
end

abort "missing #{WINAPP}" unless File.exist?(WINAPP)

section = nil
filekeys = []
rules = []
seen = Set.new

flush = lambda do
  return if section.nil? || filekeys.empty?
  return if SKIP.match?(section)
  kind = if COOKIE.match?(section)
           "cookies"
         elsif HISTORY.match?(section)
           "history"
         else
           nil
         end
  return if kind.nil?

  paths = []
  includes = []
  filekeys.each do |fk|
    next if SKIP.match?(fk)
    path, globs = parse_filekey(fk)
    next if path.empty?
    paths << path
    includes.concat(globs)
  end
  paths.uniq!
  return if paths.empty?
  id = "priv-#{kind}-#{slug(section)}"
  return unless seen.add?(id)
  h = {
    "id" => id,
    "name" => section.sub(/\s*\*\s*$/, "")[0, 100],
    "description" => "Browser #{kind} from Winapp2.ini. Off unless enabled in Junk rules → Privacy.",
    "category" => "browser-privacy",
    "platform" => "windows",
    "risk" => "high",
    "kind" => "browser",
    "default_enabled" => false,
    "min_age_days" => 0,
    "paths" => paths.first(10),
    "delete_mode" => "contents",
    "max_depth" => 8,
    "tags" => [kind, "browser", "privacy"]
  }
  includes = includes.reject(&:empty?).uniq.first(8)
  h["include_globs"] = includes unless includes.empty?
  rules << h
end

File.read(WINAPP, encoding: "UTF-8").each_line do |line|
  line = line.strip
  if line =~ /^\[(.+)\]$/
    flush.call
    section = $1
    filekeys = []
  elsif line =~ /^FileKey\d+=(.+)/i
    filekeys << $1
  end
end
flush.call

# Common Chromium/Firefox cookie+history paths on macOS/Linux (same settings tags).
[
  ["macos", "priv-cookies-chrome-mac", "Chrome cookies", "cookies",
   ["$HOME/Library/Application Support/Google/Chrome/Default/Cookies",
    "$HOME/Library/Application Support/Google/Chrome/Default/Cookies-journal"]],
  ["macos", "priv-history-chrome-mac", "Chrome history", "history",
   ["$HOME/Library/Application Support/Google/Chrome/Default/History",
    "$HOME/Library/Application Support/Google/Chrome/Default/History-journal",
    "$HOME/Library/Application Support/Google/Chrome/Default/Visited Links"]],
  ["macos", "priv-cookies-firefox-mac", "Firefox cookies", "cookies",
   ["$HOME/Library/Application Support/Firefox/Profiles"]],
  ["macos", "priv-history-firefox-mac", "Firefox history", "history",
   ["$HOME/Library/Application Support/Firefox/Profiles"]],
  ["linux", "priv-cookies-chrome-linux", "Chrome cookies", "cookies",
   ["$HOME/.config/google-chrome/Default/Cookies", "$XDG_CONFIG_HOME/google-chrome/Default/Cookies"]],
  ["linux", "priv-history-chrome-linux", "Chrome history", "history",
   ["$HOME/.config/google-chrome/Default/History", "$XDG_CONFIG_HOME/google-chrome/Default/History"]],
  ["linux", "priv-cookies-firefox-linux", "Firefox cookies", "cookies",
   ["$HOME/.mozilla/firefox"]],
  ["linux", "priv-history-firefox-linux", "Firefox history", "history",
   ["$HOME/.mozilla/firefox"]],
  ["macos", "priv-cookies-safari-mac", "Safari cookies", "cookies",
   ["$HOME/Library/Cookies"]],
  ["macos", "priv-history-safari-mac", "Safari history", "history",
   ["$HOME/Library/Safari/History.db", "$HOME/Library/Safari/History.db-wal"]],
].each do |plat, id, name, kind, paths|
  extra = {}
  extra["include_globs"] = ["**/cookies.sqlite", "**/cookies.sqlite-wal"] if id.include?("firefox") && kind == "cookies"
  extra["include_globs"] = ["**/places.sqlite", "**/places.sqlite-wal"] if id.include?("firefox") && kind == "history"
  rules << {
    "id" => id,
    "name" => name,
    "description" => "Browser #{kind}. Off unless enabled in Junk rules → Privacy.",
    "category" => "browser-privacy",
    "platform" => plat,
    "risk" => "high",
    "kind" => "browser",
    "default_enabled" => false,
    "min_age_days" => 0,
    "paths" => paths,
    "delete_mode" => "contents",
    "max_depth" => 6,
    "tags" => [kind, "browser", "privacy"]
  }.merge(extra)
end

doc = {
  "pack" => "browser-privacy",
  "description" => "Cookies and browsing history (Winapp2 + common browser DBs). Disabled unless Settings/Rules Privacy checkboxes are on. Passwords are not included.",
  "rules" => rules
}
File.write(OUT, YAML.dump(doc))
puts "wrote #{rules.size} privacy rules → #{OUT} (cookies=#{rules.count { |r| r['tags'].include?('cookies') }} history=#{rules.count { |r| r['tags'].include?('history') }})"
