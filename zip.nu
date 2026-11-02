def main [ folder: string ] {
  let pat = $env.PWD | path join "crates" ($folder);

  let output = ($env.PWD | path join $"($folder)(date now|format date '_%H_%M_%m%d%Y').zip")
  if ($pat | path type) == "dir" {
    ouch compress --format zip $pat $output
  } else {
    print $"($pat) not a directory"

  }
}