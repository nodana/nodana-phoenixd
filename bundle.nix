{ pkgs, releaseVersion }:
let
  phoenixd = pkgs.callPackage ./phoenixd.nix { };
  wrapper = pkgs.pkgsStatic.rustPlatform.buildRustPackage {
    pname = "nodana-phoenixd";
    version = releaseVersion;
    src = ./launcher;
    cargoLock.lockFile = ./launcher/Cargo.lock;
    NODANA_RELEASE = releaseVersion;
  };
in pkgs.runCommand "nodana-phoenixd-${releaseVersion}" {
  nativeBuildInputs = [ pkgs.patchelf pkgs.pax-utils ];
} ''
  mkdir -p $out/bin $out/libexec $out/lib
  cp ${wrapper}/bin/nodana-phoenixd $out/bin/
  cp ${phoenixd}/bin/phoenixd $out/libexec/
  # Resolve exactly the ELF dependency graph, using its existing pinned RPATHs.
  # Whole Nix closures can contain different builds with the same SONAME.
  lddtree -l ${phoenixd}/bin/phoenixd > libraries
  while read -r library; do
    if test "$library" != ${phoenixd}/bin/phoenixd; then
      name=$(basename "$library")
      if test -e "$out/lib/$name"; then
        cmp "$library" "$out/lib/$name"
      else
        cp -L "$library" "$out/lib/$name"
      fi
    fi
  done < libraries
  cp -L "$(patchelf --print-interpreter $out/libexec/phoenixd)" $out/lib/ld.so
  chmod u+w $out/libexec/phoenixd $out/lib/*
  patchelf --set-rpath '$ORIGIN/../lib' $out/libexec/phoenixd
  for library in $out/lib/*; do
    # The loader has no dynamic section requiring a search path.
    if test "$(basename "$library")" != ld.so && ! [[ "$(basename "$library")" == ld-linux* ]]; then
      patchelf --set-rpath '$ORIGIN' "$library"
    fi
  done
  # --version opens no wallet and never starts a networked node.
  test "$($out/bin/nodana-phoenixd --nodana-version)" = '${releaseVersion}'
  mkdir -p "$TMPDIR/version"
  HOME="$TMPDIR" PHOENIX_DATADIR="$TMPDIR/version" TZ=UTC $out/bin/nodana-phoenixd --version
''
