# 3D models

**Status: APPROVED (partial).** Static glTF triangle meshes with base-color
textures are loaded and rendered behind the 2D scene.

## Loading

- Stage DEFs read `[Model] model = <file.gltf>` relative to the stage directory.
- Storyboard DEFs read `[SceneDef] model = <file.gltf>` relative to the
  storyboard directory.
- `gltf::import` resolves the document, `.bin` buffers, and data URIs.

## Rendering

- Every triangle primitive in `mode = TRIANGLES` contributes three vertices
  with `baseColorFactor` and `TEXCOORD_0`.
- `baseColorTexture` images are decoded to RGBA and uploaded per material;
  primitives without a texture use a white 1x1 fallback. The shader is unlit:
  `texture(baseColor, uv) * baseColorFactor`.
- The mesh bounding box is centered horizontally, scaled so its largest axis
  spans 70% of the 240px screen height, and placed standing at 88% screen
  height.
- Vertices are baked into 320x240 screen space with depth `0.98`, drawn with a
  dedicated wgpu triangle pipeline before sprite/font quads. Depth testing
  keeps the 2D scene in front.
- Loading a new stage/storyboard replaces the active model; a missing or
  invalid file logs a warning and leaves no model.

## Not modeled

- Node transforms, skinning, morph targets, and glTF animations.
- PBR lighting, metallic/roughness, normal/emissive maps, and environment maps.
- `ModifyBGCtrl3D` targeting individual model nodes.
- 3D placement/offset/rotation/scale fields from the stage/storyboard DEF.
