# type2-runtime static library notices

The pinned runtime 20251108 is distributed by AppImage/type2-runtime. Its
upstream LICENSE lists musl, libfuse, squashfuse, zstd and zlib. These copies
preserve their license texts; they do not assert unmeasured binary library
version numbers. Build instructions and the libfuse patch are in the pinned
runtime source, including its pinned libfuse 3.15.0 and squashfuse 0.5.2 sources:

- https://github.com/AppImage/type2-runtime/tree/20251108
- https://github.com/AppImage/type2-runtime/blob/20251108/scripts/common/install-dependencies.sh
- https://raw.githubusercontent.com/libfuse/libfuse/fuse-3.15.0/LGPL2.txt
- https://raw.githubusercontent.com/vasi/squashfuse/0.5.2/LICENSE
- https://raw.githubusercontent.com/facebook/zstd/v1.5.7/LICENSE
- https://raw.githubusercontent.com/madler/zlib/v1.3.1/LICENSE
- https://git.musl-libc.org/cgit/musl/tree/COPYRIGHT

The musl host timed out during collection; the COPYRIGHT text was retrieved
from https://raw.githubusercontent.com/kraj/musl/v1.2.5/COPYRIGHT (source mirror).
No code from these source archives was executed. Line endings/trailing spaces
in the license copies were normalized without changing the text.

Before public distribution, retain access to corresponding runtime build
sources and relinking material for its LGPL portions; the runtime is a separate
upstream artifact, not linked into Grajek's executable.
