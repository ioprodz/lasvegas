# Cross compiling on MacOS

run

```
rustup target add armv7-unknown-linux-musleabihf
```

run

```sh
brew install arm-linux-gnueabihf-binutils
```

out:

```sh
Running `brew update --auto-update`...
==> Auto-updated Homebrew!
Updated 3 taps (pulumi/tap, homebrew/core and homebrew/cask).
==> New Formulae
blocky                dexter                enchive               flavours              hz                    imessage-exporter     ksops                 libansilove           notify                typst
==> New Casks
cursor                                                                   dehelper                                                                 hummingbird

You have 21 outdated formulae and 1 outdated cask installed.
You can upgrade them with brew upgrade
or list them with brew outdated.

==> Fetching arm-linux-gnueabihf-binutils
==> Downloading https://ghcr.io/v2/homebrew/core/arm-linux-gnueabihf-binutils/manifests/2.40
######################################################################## 100.0%
==> Downloading https://ghcr.io/v2/homebrew/core/arm-linux-gnueabihf-binutils/blobs/sha256:35c178b570359bdf49e609783af569e304ff8242f7035b5aa8ea02ed2530d0c4
==> Downloading from https://pkg-containers.githubusercontent.com/ghcr1/blobs/sha256:35c178b570359bdf49e609783af569e304ff8242f7035b5aa8ea02ed2530d0c4?se=2023-04-02T10%3A45%3A00Z&sig=OlNG5Hfn5jsEh4KLQPmp%2Bs699ZjZirACYN
######################################################################## 100.0%
==> Pouring arm-linux-gnueabihf-binutils--2.40.ventura.bottle.tar.gz
🍺  /usr/local/Cellar/arm-linux-gnueabihf-binutils/2.40: 105 files, 43.2MB
==> Running `brew cleanup arm-linux-gnueabihf-binutils`...
Disable this behaviour by setting HOMEBREW_NO_INSTALL_CLEANUP.
Hide these hints with HOMEBREW_NO_ENV_HINTS (see `man brew`).
```

run:

```sh
brew install llvm
```

out:

```
==> Downloading https://formulae.brew.sh/api/formula.jws.json
######################################################################## 100.0%
==> Downloading https://formulae.brew.sh/api/cask.jws.json

==> Fetching dependencies for llvm: six and z3
==> Fetching six
==> Downloading https://ghcr.io/v2/homebrew/core/six/manifests/1.16.0_3
######################################################################## 100.0%
==> Downloading https://ghcr.io/v2/homebrew/core/six/blobs/sha256:0dee50367c6facbfc8f65e8a82bcd3e08d43da262b1adff6ccf943ef5bfaf313
==> Downloading from https://pkg-containers.githubusercontent.com/ghcr1/blobs/sha256:0dee50367c6facbfc8f65e8a82bcd3e08d43da262b1adff6ccf943ef5bfaf313?se=2023-04-02T10%3A50%3A00Z&sig=ucfYDduy%2BvZoDEgIuEGmpjaEh4mbAtpNxO
######################################################################## 100.0%
==> Fetching z3
==> Downloading https://ghcr.io/v2/homebrew/core/z3/manifests/4.12.1
######################################################################## 100.0%
==> Downloading https://ghcr.io/v2/homebrew/core/z3/blobs/sha256:9918c8a891562b14bb69d7642a5f3cf5a79767baf78970710fd9c67e405a2f37
==> Downloading from https://pkg-containers.githubusercontent.com/ghcr1/blobs/sha256:9918c8a891562b14bb69d7642a5f3cf5a79767baf78970710fd9c67e405a2f37?se=2023-04-02T10%3A50%3A00Z&sig=QoFd3XTgqjeUwHwLMFzsz7E7saK2ItsD3%2B
######################################################################## 100.0%
==> Fetching llvm
==> Downloading https://ghcr.io/v2/homebrew/core/llvm/manifests/16.0.0
######################################################################## 100.0%
==> Downloading https://ghcr.io/v2/homebrew/core/llvm/blobs/sha256:a62b54a250911f15f8a6e7893b78f13937fdc42177b7dd0a4a8789c4af667ac9
==> Downloading from https://pkg-containers.githubusercontent.com/ghcr1/blobs/sha256:a62b54a250911f15f8a6e7893b78f13937fdc42177b7dd0a4a8789c4af667ac9?se=2023-04-02T10%3A50%3A00Z&sig=t55h4m%2BFDMPerqah35jogsPZ01Z0NV2Arx
######################################################################## 100.0%
==> Installing dependencies for llvm: six and z3
==> Installing llvm dependency: six
==> Pouring six--1.16.0_3.all.bottle.tar.gz
🍺  /usr/local/Cellar/six/1.16.0_3: 20 files, 122.4KB
==> Installing llvm dependency: z3
==> Pouring z3--4.12.1.ventura.bottle.tar.gz
🍺  /usr/local/Cellar/z3/4.12.1: 144 files, 38.3MB
==> Installing llvm
==> Pouring llvm--16.0.0.ventura.bottle.tar.gz
==> Caveats
To use the bundled libc++ please add the following LDFLAGS:
  LDFLAGS="-L/usr/local/opt/llvm/lib/c++ -Wl,-rpath,/usr/local/opt/llvm/lib/c++"

llvm is keg-only, which means it was not symlinked into /usr/local,
because macOS already provides this software and installing another version in
parallel can cause all kinds of trouble.

If you need to have llvm first in your PATH, run:
  echo 'export PATH="/usr/local/opt/llvm/bin:$PATH"' >> ~/.zshrc

For compilers to find llvm you may need to set:
  export LDFLAGS="-L/usr/local/opt/llvm/lib"
  export CPPFLAGS="-I/usr/local/opt/llvm/include"
==> Summary
🍺  /usr/local/Cellar/llvm/16.0.0: 6,779 files, 1.6GB
==> Running `brew cleanup llvm`...
Disable this behaviour by setting HOMEBREW_NO_INSTALL_CLEANUP.
Hide these hints with HOMEBREW_NO_ENV_HINTS (see `man brew`).
==> Caveats
==> llvm
To use the bundled libc++ please add the following LDFLAGS:
  LDFLAGS="-L/usr/local/opt/llvm/lib/c++ -Wl,-rpath,/usr/local/opt/llvm/lib/c++"

llvm is keg-only, which means it was not symlinked into /usr/local,
because macOS already provides this software and installing another version in
parallel can cause all kinds of trouble.

If you need to have llvm first in your PATH, run:
  echo 'export PATH="/usr/local/opt/llvm/bin:$PATH"' >> ~/.zshrc

For compilers to find llvm you may need to set:
  export LDFLAGS="-L/usr/local/opt/llvm/lib"
  export CPPFLAGS="-I/usr/local/opt/llvm/include"
```

run:

```
export DYLD_LIBRARY_PATH=/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/
```
