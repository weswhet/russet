# Apple root certificates

Russet's native signature checks trust only these roots. They were downloaded
from Apple's certificate authority pages on 2026-10-07 and match the copies in
macOS's `/System/Library/Keychains/SystemRootCertificates.keychain`. Tests pin
these SHA-256 fingerprints:

| File | Subject | Source | SHA-256 |
| --- | --- | --- | --- |
| `AppleIncRootCertificate.cer` | Apple Root CA | <https://www.apple.com/appleca/AppleIncRootCertificate.cer> | `b0b1730ecbc7ff4505142c49f1295e6eda6bcaed7e2c68c5be91b5a11001f024` |
| `AppleRootCA-G2.cer` | Apple Root CA - G2 | <https://www.apple.com/certificateauthority/AppleRootCA-G2.cer> | `c2b9b042dd57830e7d117dac55ac8ae19407d38e41d88f3215bc3a890444a050` |
| `AppleRootCA-G3.cer` | Apple Root CA - G3 | <https://www.apple.com/certificateauthority/AppleRootCA-G3.cer> | `63343abfb89a6a03ebb57e9b3f5fa7be7c4f5c756f3017b3a8c488c3653e9179` |
