# 3D models

**Status: APPROVED (partial).** glTF triangle meshes with base-color textures
and the first node animation are loaded and rendered behind the 2D scene.

## Loading

- Stage DEFs read `[Model] model = <file.gltf>` relative to the stage directory.
- Storyboard DEFs read `[SceneDef] model = <file.gltf>` relative to the
  storyboard directory.
- `gltf::import` resolves the document, `.bin` buffers, and data URIs.

## Rendering

- Every triangle primitive in `mode = TRIANGLES` contributes vertices with
  `baseColorFactor`, `TEXCOORD_0`, and `NORMAL` (default +Y).
- `baseColorTexture` images are decoded to RGBA and uploaded per material;
  primitives without a texture use a white 1x1 fallback.
- Lighting is computed per vertex before upload: a fixed directional light
  (0.3, 0.8, 0.5) plus 0.35 ambient, with `metallicFactor` reducing diffuse
  and `roughnessFactor` controlling a small highlight. The final fragment is
  `texture(baseColor, uv) * shadedColor`.
- The mesh bounding box is centered horizontally, scaled so its largest axis
  spans 70% of the 240px screen height, and placed standing at 88% screen
  height.
- Vertices are baked into 320x240 screen space with depth `0.98`, drawn with a
  dedicated wgpu triangle pipeline before sprite/font quads. Depth testing
  keeps the 2D scene in front.
- Loading a new stage/storyboard replaces the active model; a missing or
  invalid file logs a warning and leaves no model.

## Not modeled

- The first glTF animation's translation/rotation/scale channels are sampled
  (step/linear) and applied through the node hierarchy each frame. Animation
  time follows the match clock (or storyboard scene clock) in seconds.
- Skinning: `JOINTS_0`/`WEIGHTS_0` and `inverseBindMatrices` deform vertices
  with up to four weighted joints per vertex, driven by the animated node
  hierarchy.
- Morph targets: `TARGET_POSITION` deltas are blended by node/mesh default
  weights or by the first clip's `weights` channel.
- Additional animation clips.
- Full PBR BRDFs, image-based environment lighting, normal/emissive/ORM maps,
  and multiple lights.
- `ModifyBGCtrl3D` targeting individual model nodes (node transforms are
  driven by glTF animation, not stage controllers).
- 3D placement/offset/rotation/scale fields from the stage/storyboard DEF.
