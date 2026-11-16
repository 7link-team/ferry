# Report fonts

Noto Sans SC Regular and Bold (Noto Sans CJK, SIL Open Font License 1.1, see `OFL.txt`),
used by the `report.render` workflow node. The upstream `SubsetOTF/SC` files are further
subset with fontTools `pyftsubset` to keep the server binary small:

    --unicodes=U+0020-007E,U+00A0-024F,U+02B0-02FF,U+0300-036F,U+0370-03FF,U+0400-04FF,
      U+2000-206F,U+20A0-20CF,U+2100-218F,U+2190-21FF,U+2200-22FF,U+2300-23FF,U+2460-24FF,
      U+2500-257F,U+2580-259F,U+25A0-25FF,U+2600-26FF,U+2700-27BF,U+3000-303F,U+3040-30FF,
      U+3100-312F,U+31C0-31EF,U+3200-32FF,U+3300-33FF,U+4E00-9FFF,U+F900-FAFF,U+FE30-FE4F,
      U+FF00-FFEF
    --layout-features=kern,liga,locl,palt,ccmp,mark,mkmk --no-hinting

CJK Extension A (U+3400-4DBF) and later extensions are not included; such rare characters
render with a fallback box.
