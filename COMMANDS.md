# Open CAD Studio — Command Reference

Status of every standard CAD command in Open CAD Studio:

- ✅ **Implemented** — fully working
- 🔶 **Partial** — command is accepted but logic is a stub
- ❌ **Missing** — not yet implemented

---

## Draw

| Command | Alias | Description | Status |
|---|---|---|---|
| `LINE` | L | Straight line segment | ✅ |
| `PLINE` | PL | Polyline | ✅ |
| `ARC` | A | Arc | ✅ |
| `CIRCLE` | C | Circle | ✅ |
| `ELLIPSE` | EL | Ellipse | ✅ |
| `RECTANGLE` | REC | Rectangle | ✅ |
| `POLYGON` | POL | Regular polygon | ✅ |
| `XLINE` | XL | Infinite construction line | ✅ |
| `RAY` | — | One-way infinite line | ✅ |
| `SPLINE` | SPL | NURBS spline | ✅ |
| `SPLINEFIT` | FITSPLINE | Fit a spline through a polyline's points | ✅ |
| `MLINE` | ML | Multiline | ✅ |
| `POINT` | PO | Point | ✅ |
| `DONUT` | DO | Filled ring | ✅ |
| `HATCH` | H | Hatch fill | ✅ |
| `GRADIENT` | GD | Gradient fill | ✅ |
| `BOUNDARY` | BO | Boundary polyline / region | ✅ |
| `REVCLOUD` | — | Revision cloud | ✅ |
| `WIPEOUT` | WO | Wipeout mask | ✅ |
| `MTEXT` | MT | Multiline text | ✅ |
| `TEXT` | DT | Single-line text | ✅ |
| `TABLE` | — | Table entity | ✅ |
| `DIVIDE` | DIV | Divide entity into equal parts | ✅ |
| `MEASURE` | ME | Divide entity at measured intervals | ✅ |
| `3DPOLY` | — | 3D polyline | ✅ |
| `HELIX` | — | 3D helix | ✅ |
| `TRACE` | — | Thick 2D line (legacy) | ✅ |
| `SKETCH` | — | Freehand sketch | ✅ |
| `SOLID` | SO | Filled 2D shape (legacy) | ✅ |
| `MINSERT` | — | Matrix block insert | ✅ |
| `REGION` | REG | 2D closed region | ✅ |
| `FIELD` | — | Auto-updating text field | ❌ |

---

## Modify

| Command | Alias | Description | Status |
|---|---|---|---|
| `MOVE` | M | Move | ✅ |
| `COPY` | CO | Copy | ✅ |
| `ROTATE` | RO | Rotate | ✅ |
| `SCALE` | SC | Scale | ✅ |
| `MIRROR` | MI | Mirror | ✅ |
| `OFFSET` | O | Offset | ✅ |
| `TRIM` | TR | Trim | ✅ |
| `EXTEND` | EX | Extend | ✅ |
| `STRETCH` | S | Stretch | ✅ |
| `FILLET` | F | Fillet | ✅ |
| `CHAMFER` | CHA | Chamfer | ✅ |
| `ARRAY` | AR | Array | ✅ |
| `ARRAYRECT` | — | Rectangular array | ✅ |
| `ARRAYPOLAR` | — | Polar array | ✅ |
| `ARRAYPATH` | — | Path array | ✅ |
| `BREAK` | BR | Break entity | ✅ |
| `BREAKATPOINT` | — | Break at point | ✅ |
| `JOIN` | J | Join entities | ✅ |
| `EXPLODE` | X | Explode compound entity | ✅ |
| `ERASE` | E | Erase | ✅ |
| `LENGTHEN` | LEN | Lengthen / shorten | ✅ |
| `PEDIT` | PE | Edit polyline | ✅ |
| `SPLINEDIT` | SPE | Edit spline | ✅ |
| `MATCHPROP` | MA | Match properties | ✅ |
| `SCALETEXT` | — | Scale text objects | ✅ |
| `FLATTEN` | — | Flatten 3D to 2D | ✅ |
| `DRAWORDER` | DR | Draw order | ✅ |
| `ALIGN` | AL | Align | ✅ |
| `GROUP` | G | Group | ✅ |
| `UNGROUP` | UG | Ungroup | ✅ |
| `OVERKILL` | — | Remove duplicate geometry | 🔶 |
| `3DMOVE` | — | 3D move | ✅ |
| `3DROTATE` | ROTATE3D | 3D rotate (cached solid) | ✅ |
| `3DMIRROR` | MIRROR3D | 3D mirror (cached solid) | ✅ |
| `3DALIGN` | ALIGN3D | 3D align (cached solid) | ✅ |
| `3DARRAY` | ARRAY3D | 3D array | ✅ |
| `SLICE` | SL | Slice solid with plane | ✅ |
| `SUBTRACT` | SU | Subtract solids | ✅ |
| `UNION` | UNI | Union solids | ✅ |
| `INTERSECT` | IN | Intersect solids | ✅ |
| `CHAMFERSOLID` | — | Chamfer solid edge | ❌ |
| `FILLETEDGE` | — | Fillet solid edge | ❌ |

---

## Dimension

| Command | Alias | Description | Status |
|---|---|---|---|
| `DIMLINEAR` | DLI | Linear dimension | ✅ |
| `DIMALIGNED` | DAL | Aligned dimension | ✅ |
| `DIMANGULAR` | DAN | Angular dimension | ✅ |
| `DIMRADIUS` | DRA | Radius dimension | ✅ |
| `DIMDIAMETER` | DDI | Diameter dimension | ✅ |
| `DIMORDINATE` | DOR | Ordinate dimension | ✅ |
| `DIMCONTINUE` | DCO | Continue dimension | ✅ |
| `DIMBASELINE` | DBA | Baseline dimension | ✅ |
| `QDIM` | — | Quick dimension | ✅ |
| `DIMEDIT` | DED | Edit dimension text / position | ✅ |
| `DIMTEDIT` | DIMTED | Move dimension text | ✅ |
| `DIMBREAK` | DBR | Break dimension line | ✅ |
| `DIMSPACE` | DSPACE | Adjust spacing between dimensions | ✅ |
| `DIMJOGLINE` | DJL | Jog line in dimension | ✅ |
| `TOLERANCE` | TOL | Geometric tolerance | ✅ |
| `LEADER` | LE | Leader line (legacy) | ✅ |
| `MLEADER` | MLD | Multileader | ✅ |
| `MLEADERADD` | MLA | Add leader segment | ✅ |
| `MLEADERREMOVE` | MLR | Remove leader segment | ✅ |
| `MLEADERALIGN` | MLAL | Align multileaders | ✅ |
| `MLEADERCOLLECT` | MLC | Collect multileaders | ✅ |
| `DIMJOGGED` | DJO | Jogged radius dimension | ✅ |
| `DIMCENTER` | DCE | Center mark | ✅ |
| `CENTERLINE` | — | Center line | ✅ |
| `CENTERMARK` | — | Center mark on arc/circle | ✅ |
| `QLEADER` | QL | Quick leader (legacy) | ✅ |

---

## Text & Table

| Command | Alias | Description | Status |
|---|---|---|---|
| `STYLE` | ST | Text style manager | ✅ |
| `DDEDIT` | ED | Edit text | ✅ |
| `FIND` | — | Find and replace text | ✅ |
| `TABLESTYLE` | TS | Table style manager | ✅ |
| `DATALINK` | — | Link table to external spreadsheet (CSV) | ✅ |
| `ARCTEXT` | — | Text along an arc | ✅ |
| `TORIENT` | — | Orient text for readability | ✅ |
| `DATAEXTRACTION` | — | Data extraction wizard | 🔶 |
| `FIELD` | — | Auto-updating text field | ❌ |
| `SPELL` | SP | Spell check | ❌ |

---

## Layer

| Command | Alias | Description | Status |
|---|---|---|---|
| `LAYER` | LA | Layer manager | ✅ |
| `LAYOFF` | — | Turn layer off | ✅ |
| `LAYON` | — | Turn layer on | ✅ |
| `LAYFRZ` | — | Freeze layer | ✅ |
| `LAYTHW` | — | Thaw layer | ✅ |
| `LAYLCK` | — | Lock layer | ✅ |
| `LAYULK` | — | Unlock layer | ✅ |
| `LAYMCUR` | — | Make object's layer current | ✅ |
| `LAYMATCH` | — | Match layer of selected object | ✅ |
| `VPLAYER` | — | Viewport layer control | ✅ |
| `LINETYPE` | LT | Linetype manager | ✅ |
| `LTSCALE` | — | Global linetype scale | ✅ |
| `LAYISO` | — | Isolate layer | ✅ |
| `LAYUNISO` | — | End layer isolation | ✅ |
| `LAYDEL` | — | Delete layer | ✅ |
| `LAYMRG` | — | Merge layers | ✅ |
| `LAYERSTATE` | LAS | Save / restore layer states | ✅ |
| `LAYWALK` | — | Walk through layers | ❌ |
| `LAYLOCKFADECTL` | — | Locked layer fading control | ❌ |

---

## Block & Reference

| Command | Alias | Description | Status |
|---|---|---|---|
| `BLOCK` | B | Define block | ✅ |
| `INSERT` | I | Insert block | ✅ |
| `MINSERT` | — | Matrix block insert | ✅ |
| `WBLOCK` | W | Write block to file | ✅ |
| `XATTACH` | XA | Attach external reference | ✅ |
| `XREF` | XR | External reference manager | ✅ |
| `XRELOAD` | — | Reload external reference | ✅ |
| `XCLIP` | XC | Clip external reference | ✅ |
| `REFEDIT` | — | Edit reference in-place | ✅ |
| `REFCLOSE` | — | Close reference edit | ✅ |
| `BEDIT` | BE | Block editor | ✅ |
| `BASE` | — | Set drawing base point | ✅ |
| `NCOPY` | — | Copy nested objects out of a block | ✅ |
| `ATTDEF` | ATT | Define attribute | ✅ |
| `ATTEDIT` | ATE | Edit attribute | ✅ |
| `ATTEXT` | — | Extract attributes (legacy) | ✅ |
| `ATTSYNC` | — | Synchronize attribute definitions | ✅ |
| `BLOCKPALETTE` | — | Multi-view block palette (command-line list) | 🔶 |
| `ATTMAN` | `BATTMAN` | Block attribute manager (opens the attribute editor) | ✅ |
| `XBIND` | XB | Bind xref elements to drawing | ❌ |
| `XOPEN` | — | Open xref for editing | ✅ |
| `BSAVE` | — | Save block in editor | ❌ |
| `BCLOSE` | — | Close block editor | ❌ |

---

## 3D Modeling

| Command | Alias | Description | Status |
|---|---|---|---|
| `BOX` | — | Box solid | ✅ |
| `SPHERE` | — | Sphere solid | ✅ |
| `CYLINDER` | — | Cylinder solid | ✅ |
| `CONE` | — | Cone solid | ✅ |
| `WEDGE` | — | Wedge solid | ✅ |
| `TORUS` | — | Torus solid | ✅ |
| `PYRAMID` | PYR | Pyramid solid | ✅ |
| `POLYSOLID` | — | Wall-like solid | ✅ |
| `EXTRUDE` | EXT | Extrude profile | ✅ |
| `PRESSPULL` | — | Push / pull a closed boundary | ✅ |
| `THICKEN` | — | Thicken a closed profile to a solid | ✅ |
| `REVOLVE` | REV | Revolve profile around axis | ✅ |
| `SWEEP` | — | Sweep profile along path | ✅ |
| `LOFT` | — | Loft between profiles | ✅ |
| `MASSPROP` | — | Mass properties | ✅ |
| `EXPORTSTEP` | — | Export to STEP | ✅ |
| `EXPORTSTL` | — | Export to STL | ✅ |
| `SLICE` | SL | Slice solid with plane | ✅ |
| `SUBTRACT` | SU | Subtract solids | ✅ |
| `UNION` | UNI | Union solids | ✅ |
| `INTERSECT` | IN | Intersect solids | ✅ |
| `INTERFERE` | INF | Interference solid from overlap | ✅ |
| `SECTION` | — | Cross-section outline of a solid | ✅ |
| `CONVTOSOLID` | — | Convert to solid | ❌ |
| `CONVTOSURFACE` | — | Convert to surface | ✅ |
| `SECTIONPLANE` | — | Section plane object | ❌ |
| `FLATSHOT` | — | 2D view from 3D | ✅ |

---

## View & Navigation

| Command | Alias | Description | Status |
|---|---|---|---|
| `ZOOM` | Z | Zoom | ✅ |
| `LIMITS` | — | Drawing/grid limits and point checking | ✅ |
| `PAN` | P | Pan | ✅ |
| `ORBIT` | 3DO | 3D orbit | ✅ |
| `VPORTS` | — | Viewport configuration | ✅ |
| `VPJOIN` | — | Join viewports | ✅ |
| `SYNCPVIEWPORTS` | VPSYNC | Sync viewport display settings | ✅ |
| `MSPACE` | MS | Switch to model space | ✅ |
| `PSPACE` | PS | Switch to paper space | ✅ |
| `MVIEW` | MV | Model view in layout | ✅ |
| `UCSICON` | — | Toggle UCS icon | ✅ |
| `VIEW` | V | Named views manager | ✅ |
| `PLAN` | — | Switch to plan view | ✅ |
| `NAVVCUBE` | — | Toggle ViewCube | 🔶 |
| `NAVBAR` | — | Toggle navigation bar | 🔶 |
| `TOOLPALETTES` | — | Tool palettes panel | 🔶 |
| `PROPERTIES` | PR | Properties palette | 🔶 |
| `SHEETSET` | SSM | Sheet set manager | 🔶 |
| `FILETAB` | — | Toggle file tabs | 🔶 |
| `LAYOUTTAB` | — | Toggle layout tabs | 🔶 |
| `DVIEW` | DV | Dynamic view (legacy) | ❌ |
| `NAVSWHEEL` | — | Steering wheel | ❌ |
| `RENDER` | RR | Render | ❌ |
| `RENDERPRESETS` | — | Render presets | ❌ |
| `LIGHT` | — | Add scene light | ❌ |
| `SUNPROPERTIES` | — | Sun light settings | ❌ |
| `MATERIALS` | MAT | Material editor | ❌ |
| `VISUALSTYLES` | — | Apply a built-in visual style (no custom-style manager) | 🔶 |
| `HIDE` | HI | Hidden-line regeneration | ✅ |
| `VPMAX` | — | Maximize viewport | ❌ |
| `VPMIN` | — | Restore viewport | ❌ |

---

## Inquiry

| Command | Alias | Description | Status |
|---|---|---|---|
| `AREA` | — | Calculate area | ✅ |
| `MASSPROP` | — | Mass properties | ✅ |
| `QSELECT` | — | Quick select | ✅ |
| `STATUS` | — | Drawing status | ✅ |
| `COUNT` | — | Count objects | ✅ |
| `DIST` | DI | Distance between two points | ✅ |
| `ID` | — | Point coordinate | ✅ |
| `LIST` | LI | List object data | ✅ |
| `DBLIST` | — | List all objects | ✅ |
| `MEASUREGEOM` | — | Measure distance / angle / area | ✅ |
| `CAL` | — | Command-line calculator | ✅ |
| `QUICKCALC` | QC | Quick calculator | ✅ |

---

## File & Plot

| Command | Alias | Description | Status |
|---|---|---|---|
| `NEW` | — | New drawing | ✅ |
| `OPEN` | — | Open drawing | ✅ |
| `SAVE` | — | Save | ✅ |
| `SAVEAS` | — | Save as | ✅ |
| `QSAVE` | — | Quick save | ✅ |
| `SAVEALL` | — | Save all open drawings | ✅ |
| `PLOT` | — | Print / plot | ✅ |
| `EXPORT [path]` | — | Export to PDF (save dialog, or straight to `path`) | ✅ |
| `EXPORTPDF [path]` | — | Export to PDF (save dialog, or straight to `path`) | ✅ |
| `PAGESETUP` | — | Page setup | ✅ |
| `PLOTSTYLE` | — | Plot style | ✅ |
| `PURGE` | PU | Purge unused items | ✅ |
| `QUIT` | — | Exit application | ✅ |
| `RECOVER` | — | Recover damaged drawing | ❌ |
| `CLOSE` | — | Close drawing | ✅ |
| `ARCHIVE` | — | Package drawing + references into a folder | ✅ |
| `ETRANSMIT` | — | Transmittal package (folder) | ✅ |

---

## Manage & Customize

| Command | Alias | Description | Status |
|---|---|---|---|
| `RENAME` | — | Rename named objects | ✅ |
| `LINETYPE` | LT | Linetype manager | ✅ |
| `PLOTSTYLEEDITOR` | — | Plot style editor | ✅ |
| `MLEADERSTYLE` | — | Multileader style manager | ✅ |
| `DIMSTYLE` | D | Dimension style manager | ✅ |
| `CUI` | — | Customize user interface | ✅ |
| `CUIIMPORT` | — | Import customization file | ✅ |
| `CUIEXPORT` | — | Export customization file | ✅ |
| `ALIASEDIT` | — | Edit command aliases | ✅ |
| `OPTIONS` | OP | Application settings (drafting) | ✅ |
| `OBJECTSCALE` | — | Mark objects annotative | ✅ |
| `SCRIPT` | SCR | Run script file | ✅ |
| `AUDIT` | — | Audit drawing integrity | 🔶 |
| `OVERKILL` | — | Remove duplicate geometry | 🔶 |
| `FINDNONPURGEABLE` | — | Find non-purgeable items | 🔶 |
| `XBIND` | — | Bind xref elements | ❌ |
| `HYPERLINK` | — | Insert hyperlink | ✅ |
| `DBCONNECT` | — | Connect to external database | ❌ |
| `APPLOAD` | — | Load application (LISP / ARX) | ❌ |
| `NETLOAD` | — | Load .NET plug-in | ❌ |
| `ACTRECORD` | — | Record action macro | ❌ |
| `ACTMANAGER` | — | Action macro manager | ❌ |

---

## Summary

| Category | Total | ✅ Done | 🔶 Partial | ❌ Missing |
|---|---|---|---|---|
| Draw | 32 | 31 | 0 | 1 |
| Modify | 42 | 39 | 1 | 2 |
| Dimension | 26 | 26 | 0 | 0 |
| Text & Table | 10 | 7 | 1 | 2 |
| Layer | 19 | 17 | 0 | 2 |
| Block & Reference | 23 | 18 | 2 | 3 |
| 3D Modeling | 27 | 25 | 0 | 2 |
| View & Navigation | 33 | 16 | 8 | 9 |
| Inquiry | 12 | 12 | 0 | 0 |
| File & Plot | 17 | 16 | 0 | 1 |
| Manage & Customize | 22 | 13 | 3 | 6 |
| **Total** | **263** | **220** | **15** | **28** |

> Counts include commands listed under more than one category (e.g. `SLICE`, `HELIX`,
> `MINSERT`, `SUBTRACT`/`UNION`/`INTERSECT` appear in both their 2D and 3D groups).
>
> Still out of scope: `POINTCLOUDATTACH`/`RECAP` (proprietary binary point-cloud parser
> plus a GPU point renderer) and `UNDERLAYLAYERS`/`UOSNAP` (PDF/DWF internal-layer
> parsing) — each needs an external dependency and real test files.
