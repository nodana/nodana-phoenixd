{ stdenvNoCC, fetchurl, unzip, autoPatchelfHook, stdenv, zstd, zlib, libxcrypt-legacy }:
assert stdenv.hostPlatform.isAarch64;
stdenvNoCC.mkDerivation {
  pname = "phoenixd";
  version = "0.9.3";
  src = fetchurl {
    url = "https://github.com/ACINQ/phoenixd/releases/download/v0.9.3/phoenixd-0.9.3-linux-arm64.zip";
    sha256 = "45400c0981ec8d1e3ef33889195ce5604cf9d9ed682a5c210e1e702cd6bde7cf";
  };
  nativeBuildInputs = [ unzip autoPatchelfHook ];
  buildInputs = [ stdenv.cc.cc.lib zstd zlib libxcrypt-legacy ];
  installPhase = ''
    install -Dm755 phoenixd $out/bin/phoenixd
  '';
}
