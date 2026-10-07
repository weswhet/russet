# Certificate bundle

`cacert.pem` is the unmodified certificate bundle from certifi 2025.10.5,
matching the pinned Python reference. It contains 147 certificates.
SHA-256: `2089fc5a25836401faee99f722460b01b393999746a0fb817943666d6ecc0458`.

The bundle is embedded in the native executable. `URLDownloaderPython` adds it
to the reference platform's selected default trust locations and any explicit
`SSL_CERT_FILE`. On POSIX, `SSL_CERT_FILE` and `SSL_CERT_DIR` independently
replace the default file and hashed-directory locations. On Windows, the CA
and ROOT certificate stores also contribute server-authentication certificates.

The selected macOS and Linux curl backends perform hashed-directory lookup.
An empty `SSL_CERT_DIR` disables directory trust. Windows Schannel does not
support CApath; the native implementation imports correctly hashed entries
using the `openssl x509 -subject_hash` system tool. The same tool handles
numbered-index gaps on POSIX, which the pinned Python runtime accepts but some
curl backends omit. Arbitrary PEM filenames and incorrect subject hashes do
not add trust. If this import is needed and `openssl` is unavailable, execution
fails with an explicit dependency error. The import does not change host trust
settings.

Source: https://pypi.org/project/certifi/2025.10.5/
License: Mozilla Public License 2.0; see LICENSE.
