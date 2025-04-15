{
  description = "Blue RDMA Driver";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        kernel = pkgs.linuxPackages_6_12.kernel;
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rdma-core
            cmake
            docutils
            pandoc
            pkg-config
            python3
            ethtool
            iproute2
            libnl
            perl
            udev

            gnumake
            gcc
            kernel.dev
          ];

          KERNEL_DIR = "${kernel.dev}/lib/modules/${kernel.modDirVersion}/build";
        };
      }
    );
}
