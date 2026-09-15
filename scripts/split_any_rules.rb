#!/usr/bin/env ruby
# frozen_string_literal: true

# Split platform: any rules into macos / linux / windows by path hints.
# Rewrites YAML in place (categories/, platforms/, generated/).

require "yaml"
require "fileutils"

ROOT = File.expand_path("..", __dir__)
RULES = File.join(ROOT, "rules")

def classify_path(path)
  s = path.to_s
  return :windows if s.match?(/(%|\\|C:\/|Program Files|LOCALAPPDATA|APPDATA|USERPROFILE|WINDIR|PROGRAMDATA|SystemDrive|\$Recycle)/i)
  return :macos if s.match?(%r{Library/|/Applications|Photos Library|\.savedState|HTTPStorages|Containers/|QuickLook|CoreSimulator|Xcode|Mobile Documents|CloudStorage})
  return :linux if s.match?(%r{\$XDG_|/var/|/usr/|/etc/|\.linuxbrew|\.var/app|flatpak|/tmp\b})
  # Shared Unix home layouts (exist on both macOS and Linux)
  return :unix if s.match?(%r{\$HOME/|\$CACHE|\$TMPDIR|\$TEMP|~/})
  :unix
end

def clone_rule(r, platform, paths, id)
  h = {}
  r.each do |k, v|
    next if k == "platform" || k == "paths" || k == "id"
    h[k] = v
  end
  h["id"] = id
  h["platform"] = platform
  h["paths"] = paths
  h
end

def split_rule(r)
  return [r] unless (r["platform"] || "any").to_s == "any"

  buckets = { macos: [], linux: [], windows: [] }
  Array(r["paths"]).each do |p|
    case classify_path(p)
    when :windows then buckets[:windows] << p
    when :macos then buckets[:macos] << p
    when :linux then buckets[:linux] << p
    when :unix
      buckets[:macos] << p
      buckets[:linux] << p
    end
  end

  non_empty = buckets.select { |_, paths| !paths.empty? }
  if non_empty.empty?
    # Fallback: treat as unix shared
    return [
      clone_rule(r, "macos", Array(r["paths"]), "#{r["id"]}-macos"),
      clone_rule(r, "linux", Array(r["paths"]), "#{r["id"]}-linux")
    ]
  end

  if non_empty.size == 1
    plat, paths = non_empty.first
    return [clone_rule(r, plat.to_s, paths.uniq, r["id"])]
  end

  non_empty.map do |plat, paths|
    clone_rule(r, plat.to_s, paths.uniq, "#{r["id"]}-#{plat}")
  end
end

def ensure_unique(rules)
  seen = {}
  out = []
  rules.each do |r|
    id = r["id"].to_s
    base = id
    n = 2
    while seen[id]
      id = "#{base}-#{n}"
      n += 1
    end
    r = r.merge("id" => id) if id != r["id"]
    seen[id] = true
    out << r
  end
  out
end

stats = { files: 0, any_in: 0, rules_out: 0, by_plat: Hash.new(0) }

Dir.glob(File.join(RULES, "**", "*.yaml")).sort.each do |file|
  next if File.basename(file).start_with?("_")
  data = YAML.load_file(file) || {}
  rules = data["rules"] || []
  next if rules.empty?

  any_count = rules.count { |r| (r["platform"] || "any").to_s == "any" }
  next if any_count.zero? && !file.include?("/generated/any")

  stats[:files] += 1
  stats[:any_in] += any_count

  split = rules.flat_map { |r| split_rule(r) }
  split = ensure_unique(split)
  # Drop empty-path rules just in case
  split.reject! { |r| Array(r["paths"]).empty? }

  if file.include?("/generated/any")
    # Redistribute into generated/{macos,linux,windows}-split.yaml later — write by platform files
    FileUtils.rm_f(file)
    by = Hash.new { |h, k| h[k] = [] }
    split.each { |r| by[r["platform"]] << r }
    by.each do |plat, list|
      out = File.join(File.dirname(file), "from-any-#{plat}.yaml")
      doc = {
        "pack" => "generated-from-any-#{plat}",
        "description" => "Split from former platform:any rules (#{plat}).",
        "rules" => list
      }
      File.write(out, YAML.dump(doc))
      puts "  wrote #{out} (#{list.size})"
    end
  else
    data["rules"] = split
    data["description"] = data["description"].to_s.sub(/any/i, "split-by-os") if data["description"]
    File.write(file, YAML.dump(data))
    puts "  rewrote #{file} (#{rules.size} → #{split.size})"
  end

  split.each { |r| stats[:by_plat][r["platform"]] += 1 }
  stats[:rules_out] += split.size
end

puts "files touched≈#{stats[:files]} any_in=#{stats[:any_in]}"
puts "platform tallies from rewritten batches: #{stats[:by_plat].inspect}"
puts "note: run expand script afterward if you want a clean generated/ mix"
