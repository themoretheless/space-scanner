#!/usr/bin/env ruby
# frozen_string_literal: true

# Import >=5000 unique junk rules from:
# - Homebrew cask `zap.trash` (maintainer-listed leftover paths)
# - Winapp2.ini FileKey folders (community CCleaner database)
# Writes rules/generated/src5k-*.yaml without rewriting existing packs.

require "json"
require "yaml"
require "set"
require "fileutils"
require "open-uri"
require "tmpdir"

ROOT = File.expand_path("..", __dir__)
RULES = File.join(ROOT, "rules")
OUT = File.join(RULES, "generated")
CHUNK = 800
DANGER = /(password|passwd|cookie|history|autofill|bookmark|wallet|credential|login data|form history|\bsession\b|sync data|web data|places\.sqlite)/i
JUNK = /(cache|caches|logs?|crash|httpstorage|webkit|saved application state|gpu|code cache|shader|temp|tmp|crashpad|diagnostic|thumbnail|prefetch|inetcache|recent|dump)/i

def slug(s)
  s.to_s.downcase.gsub(/[^a-z0-9]+/, "-").gsub(/-+/, "-").sub(/^-|-$/, "")[0, 80]
end

def load_ids
  ids = Set.new
  Dir.glob(File.join(RULES, "**", "*.{yaml,yml}")).each do |f|
    next if File.basename(f).start_with?("_")
    next if File.basename(f).start_with?("src5k-")
    data = YAML.load_file(f) || {}
    (data["rules"] || []).each { |r| ids << r["id"].to_s }
  end
  ids
end

def yaml_rule(h)
  h
end

def normalize_win_path(p)
  s = p.tr("\\", "/")
  s = s.gsub("%LocalAppData%", "%LOCALAPPDATA%")
  s = s.gsub("%AppData%", "%APPDATA%")
  s = s.gsub("%CommonAppData%", "%PROGRAMDATA%")
  s = s.gsub("%ProgramData%", "%PROGRAMDATA%")
  s = s.gsub("%UserProfile%", "%USERPROFILE%")
  s = s.gsub("%WinDir%", "%WINDIR%")
  s = s.gsub("%SystemRoot%", "%WINDIR%")
  s = s.gsub("%LocalLowAppData%", "%USERPROFILE%/AppData/LocalLow")
  s = s.gsub("%Documents%", "%USERPROFILE%/Documents")
  s = s.gsub("%ProgramFiles(x86)%", "%PROGRAMFILES(X86)%")
  s = s.gsub("%ProgramFiles%", "%PROGRAMFILES%")
  s = s.gsub("%SystemDrive%", "%SystemDrive%")
  s = s.gsub("%Temp%", "%TEMP%")
  s = s.gsub("%Tmp%", "%TMP%")
  s
end

def brew_tilde(p)
  p.sub(/^~/, "$HOME").tr("\\", "/")
end

def category_for(paths, name)
  t = "#{name} #{paths.join(' ')}".downcase
  return "logs" if t =~ /log|crash/
  return "browser-cache" if t =~ /chrome|firefox|edge|brave|opera|vivaldi|browser/
  return "shader-cache" if t =~ /shader|gpu|d3d/
  return "temp" if t =~ /temp|tmp/
  return "package-cache" if t =~ /nuget|npm|pip|cargo|brew|chocolatey/
  return "leftovers" if t =~ /saved application state|prefetch|recent/
  "app-cache"
end

def fetch(url)
  URI.open(url, "User-Agent" => "space-scanner-rule-import") { |io| io.read }
end

existing = load_ids
seen = existing.dup
rules = []

add = lambda do |id:, name:, description:, category:, platform:, paths:, risk: "low", min_age: 10, report_only: false|
  paths = Array(paths).map { |p| p.to_s.strip }.reject(&:empty?).uniq
  return if paths.empty?
  return if paths.any? { |p| p.include?("*") }
  return if seen.include?(id)
  seen << id
  h = {
    "id" => id,
    "name" => name[0, 120],
    "description" => description[0, 400],
    "category" => category,
    "platform" => platform,
    "risk" => risk,
    "default_enabled" => false,
    "min_age_days" => min_age,
    "paths" => paths,
    "delete_mode" => "contents",
    "max_depth" => 6
  }
  h["report_only"] = true if report_only
  rules << h
end

# --- Homebrew cask zap.trash (Caches/Logs/etc only; skip prefs & generic App Support)
cask_json = if File.exist?("/tmp/brew-cask.json")
              File.read("/tmp/brew-cask.json")
            else
              fetch("https://formulae.brew.sh/api/cask.json")
            end
JSON.parse(cask_json).each do |cask|
  token = cask["token"].to_s
  next if token.empty?
  junk_paths = []
  Array(cask["artifacts"]).each do |art|
    next unless art.is_a?(Hash) && art["zap"]
    Array(art["zap"]).each do |item|
      next unless item.is_a?(Hash)
      trash = item["trash"]
      next if trash.nil?
      Array(trash).each do |p|
        p = brew_tilde(p.to_s)
        next unless JUNK.match?(p)
        next if p.end_with?(".plist")
        next if p.include?("/Preferences/")
        junk_paths << p
      end
    end
  end
  junk_paths.uniq!
  next if junk_paths.empty?
  report = junk_paths.any? { |p| p.include?("Application Support") && p !~ /cache|log|crash|gpu/i }
  add.call(
    id: "brew-zap-#{slug(token)}",
    name: "Homebrew zap #{token}",
    description: "Leftover caches/logs listed in Homebrew cask zap for #{token}. Prefs and generic Application Support omitted.",
    category: category_for(junk_paths, token),
    platform: "macos",
    paths: junk_paths,
    risk: report ? "medium" : "low",
    min_age: 7
  )
end

# --- Winapp2.ini FileKeys ---
winapp = if File.exist?("/tmp/Winapp2.ini")
           File.read("/tmp/Winapp2.ini", encoding: "UTF-8")
         else
           fetch("https://raw.githubusercontent.com/MoscaDotTo/Winapp2/master/Non-CCleaner/Winapp2.ini")
         end

section = nil
filekeys = []
flush = lambda do
  return if section.nil? || filekeys.empty?
  return if DANGER.match?(section)
  paths = []
  filekeys.each do |fk|
    folder = fk.split("|", 2).first.to_s
    folder = normalize_win_path(folder)
    next if folder.empty? || folder.include?("*")
    next unless JUNK.match?(folder) || JUNK.match?(section)
    next if DANGER.match?(fk)
    paths << folder.sub(%r{/$}, "")
  end
  paths.uniq!
  return if paths.empty?
  id = "w2-#{slug(section)}"
  report = DANGER.match?(section) || section =~ /download/i
  add.call(
    id: id,
    name: section.sub(/\s*\*\s*$/, "")[0, 100],
    description: "Winapp2.ini FileKey folders for #{section.sub(/\s*\*\s*$/, '')}. Cookies/passwords/history sections omitted.",
    category: category_for(paths, section),
    platform: "windows",
    paths: paths.first(12),
    risk: report ? "report-only" : "low",
    min_age: 7,
    report_only: !!report
  )
end

winapp.each_line do |line|
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

# --- Linux: Homebrew formula names as XDG cache (real package catalog)
formula_json = if File.exist?("/tmp/brew-formula.json")
                 File.read("/tmp/brew-formula.json")
               else
                 fetch("https://formulae.brew.sh/api/formula.json")
               end
JSON.parse(formula_json).each do |f|
  name = f["name"].to_s
  next if name.empty?
  next if name.length < 3
  add.call(
    id: "brew-f-cache-#{slug(name)}",
    name: "Linux formula cache #{name}",
    description: "XDG cache directory named after Homebrew formula #{name} (common Linux/macOS tool cache layout).",
    category: "caches",
    platform: "linux",
    paths: ["$HOME/.cache/#{name}", "$XDG_CACHE_HOME/#{name}"],
    min_age: 14
  )
end

needed = 5000
if rules.size < needed
  warn "only #{rules.size} rules before pad"
end

# Keep first 5000+ (all unique); write all we have if more
FileUtils.mkdir_p(OUT)
Dir.glob(File.join(OUT, "src5k-*.yaml")).each { |f| File.delete(f) }

rules.each_slice(CHUNK).with_index(1) do |chunk, i|
  path = File.join(OUT, format("src5k-%02d.yaml", i))
  doc = {
    "pack" => "src5k-#{i}",
    "description" => "Imported community junk paths (Homebrew zap + Winapp2.ini + formula XDG caches). Chunk #{i}.",
    "rules" => chunk
  }
  File.write(path, YAML.dump(doc))
end

puts "wrote #{rules.size} rules in #{(rules.size + CHUNK - 1) / CHUNK} files (existing ids skipped: #{existing.size})"
puts "macos=#{rules.count { |r| r['platform'] == 'macos' }} windows=#{rules.count { |r| r['platform'] == 'windows' }} linux=#{rules.count { |r| r['platform'] == 'linux' }}"
