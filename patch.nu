def main [prefix: string = ""] {
  let dl = $env.USERPROFILE | path join "Downloads"
  let matches = ls $dl
    | get name
    | where { |p| ($p | path basename) starts-with $prefix }

  if ($matches | is-empty) {
    error make { msg: $"No file starting with '($prefix)' in ($dl)" }
  }

  let choice = $matches
    | each { |p| { name: ($p | path basename), path: $p } }
    | input list --fuzzy --display name "Select a file:"

  if $choice == null { return }

  let patch = $choice.path
  cp $patch .
  git apply ($patch | path basename)
  print "Done..."
}
