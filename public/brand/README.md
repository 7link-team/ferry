# Shared brand icon

The user-supplied `logo.png` is the unmodified 1254×1254 RGBA master. It replaces the previous vector design in login, navigation and browser icons. No background, crop or new artwork is added. The configured site name remains the visible and accessible product name.

| Asset | Use |
| --- | --- |
| `logo.png` | Original master, unchanged |
| `logo-256.png`, `logo-512.png` | Login and shared navigation at 1× / 2× density |
| `logo-16.png`, `logo-32.png`, `logo.ico` | Browser icons, complete user artwork |
| `logo-180.png` | Touch icon |

Run `python scripts/export-brand-icon.py` with Pillow to reproduce PNG and six-size ICO exports. Alpha transparency is retained. Small-size exports use the same complete artwork; they are not redrawn. The source is not enlarged or described as a native 2048px image.

The login mark displays at 128px on phones and 160px from the small-screen breakpoint; navigation retains its compact size.

Native desktop and mobile application icons use this same master. Regenerate
them with `npm run brand:icons`; `npm run brand:icons:check` checks all native
exports. See `desktop/icons/README.md` for native project synchronization.
