dev:
    dx serve --platform linux --hot-patch --features hot-patch
dev-release:
    dx serve --platform linux --release --hot-patch --features hot-patch
release:
    dx serve --platform linux --release 
# Hot-patch a vse-ui example, e.g. `just example pager`
example name:
    dx serve --platform linux --hot-patch --features hot-patch -p vse-ui --example {{name}}
