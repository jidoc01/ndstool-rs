# Third-party provenance and licensing

The MIT grant in LICENSE applies only to original contributions for which
ndstool-rs contributors hold the relevant rights. It does not grant rights
held by third parties, remove existing notices, or establish that the complete
program may be distributed under MIT alone.

## Upstream ndstool

This project began as a Rust porting effort based on
[devkitPro/ndstool](https://github.com/devkitPro/ndstool).
The inspected upstream checkout contains the GNU GPL version 3 in COPYING.
Upstream-derived protected code, if present, remains subject to its applicable
license; the MIT grant does not override those requirements. A code-provenance
review has not yet established which portions are independent implementations
and which are translations or adaptations. Distribution requirements for the
combined program remain unresolved pending that review.

## Console-specific data

`src/crypto.rs` contains a 0x1048-byte KEY1 initialization table matching the
table in upstream `source/encryption.cpp`. Technical references identify DS
KEY1 initialization data in ARM7 BIOS at offset 0x30. Matching that data alone
does not establish authorship by ndstool or GPL coverage of the data itself.
Nor does BIOS origin establish public-domain status or redistribution permission.
The provenance and redistribution basis for embedded console-specific data,
including the logo in `src/logo.rs`, require separate review.

References:

- [Upstream license](https://github.com/devkitPro/ndstool/blob/master/COPYING)
- [melonDS KEY1 initialization](https://github.com/melonDS-emu/melonDS/blob/master/src/NDSCart.cpp)
- [GNU FAQ: translating code](https://www.gnu.org/licenses/gpl-faq.html#TranslateCode)

Do not interpret the MIT grant as an MIT-only clearance of the complete
repository. Existing third-party rights and notices are retained.
