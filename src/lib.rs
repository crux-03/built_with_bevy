//! 'Built with Bevy' Splashscreen
//!
//! Add [`BevySplashscreenPlugin`] to your app, then:
//!
//! - `commands.trigger(StartBevySplashscreen)` to play the splash.
//! - `commands.trigger(SkipBevySplashscreen)` to end it early.
//! - `app.add_observer(|_: On<BevySplashscreenEnded>| { })` to react to end.

use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::RenderLayers,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::{
        ExtractSchedule, MainWorld, Render, RenderApp, RenderSystems,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        render_resource::PipelineCache,
        view::{ExtractedWindow, ViewTarget},
    },
};
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
    StrokeVertex, VertexBuffers, math::point, path::Path as LyonPath,
};

const BIRD_SVG: &str = include_str!("../assets/bird-0.svg");
const BUILT_SVG: &str = include_str!("../assets/built.svg");
const WITH_SVG: &str = include_str!("../assets/with.svg");
const BEVY_TEXT_SVG: &str = include_str!("../assets/bevy_text.svg");

const BIRD_SOURCE_FILL: [u8; 3] = [0xec, 0xec, 0xec]; // EKEKEKEKEK, I think a cat got in here
const BIRD_SOURCE_BG_FILL: [u8; 3] = [0x23, 0x23, 0x26];
const BEVY_TEXT_SOURCE_FILL: [u8; 3] = [0xec, 0xec, 0xec];
const BUILT_TEXT_SOURCE_FILL: [u8; 3] = [0x78, 0x78, 0x7f];
const WITH_TEXT_SOURCE_FILL: [u8; 3] = [0x78, 0x78, 0x7f];

// Max distance (in svg pixels) between a curve and its tessellated approximation.
const TESSELLATION_TOLERANCE: f32 = 0.01;
const BIRD_NAMES: [&str; 3] = ["Birb 0 (front)", "Birb 1 (middle)", "Birb 2 (back)"];

const BIRD_SLIDE_OFFSET: f32 = 20.0;

/// Customization options for the splashscreen
#[derive(Resource)]
pub struct BevySplashscreenOptions {
    /// Set the background for the splashscreen.
    ///
    /// Defaults to a darkish grey: #232326
    pub background_color: Color,
    /// The splashscreen uses a separate 2D camera to display over all others.
    ///
    /// This sets the camera ordering for it.
    ///
    /// Defaults to 9999.
    pub splash_camera_order: isize,
    /// Render layer used by the splash camera and its meshes.
    ///
    /// Defaults to 31.
    pub splash_render_layer: usize,
    /// Colors of the bevy birbs, front to back.
    ///
    /// Defaults to:
    /// [#ececec /* cat got in here */, #b2b2b2, #787878]
    pub bird_colors: [Color; 3],

    /// Color of the "BEVY" text.
    ///
    /// Defaults to #ececec
    pub bevy_text_color: Color,

    /// Color of the "built" text.
    ///
    /// Defaults to #78787f
    pub built_text_color: Color,

    /// Color of the "with" text.
    ///
    /// Defaults to #78787f
    pub with_text_color: Color,

    // Animation options
    // TODO: Maybe worthwhile making it a bit more granular, but this is good enough for now.
    /// How long each fade takes.
    ///
    /// Defaults to 0.6s
    pub fade_duration: f32,
    /// How long each keyframe takes.
    ///
    /// Defaults to 0.7s
    pub keyframe_duration: f32,
    /// How long the full splashscreen holds before it starts fading.
    ///
    /// Defaults to 1.1s
    pub hold_duration: f32,
}

impl Default for BevySplashscreenOptions {
    fn default() -> Self {
        Self {
            background_color: Color::srgb_u8(0x23, 0x23, 0x26),
            splash_camera_order: 9999,
            splash_render_layer: 31,
            bird_colors: [
                Color::srgb_u8(0xec, 0xec, 0xec),
                Color::srgb_u8(0xb2, 0xb2, 0xb2),
                Color::srgb_u8(0x78, 0x78, 0x78),
            ],

            bevy_text_color: Color::srgb_u8(0xec, 0xec, 0xec),
            built_text_color: Color::srgb_u8(0x78, 0x78, 0x7f),
            with_text_color: Color::srgb_u8(0x78, 0x78, 0x7f),

            fade_duration: 0.6,
            keyframe_duration: 0.7,
            hold_duration: 1.1,
        }
    }
}

/// Add to your project to set up the splashscreen.
///
/// `commands.trigger(StartBevySplashscreen)` to play the splash.
/// `commands.trigger(SkipBevySplashscreen)` to end it early.
/// `app.add_observer(|_: On<BevySplashscreenEnded>| { })` to react to end.
#[derive(Default)]
pub struct BevySplashscreenPlugin;

impl Plugin for BevySplashscreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BevySplashscreenOptions>()
            .init_resource::<RenderReadiness>()
            .add_plugins(ExtractComponentPlugin::<SplashCamera>::default())
            .register_type::<Fade>()
            .register_type::<KeyframeInterp>()
            .register_type::<KeyFrame>()
            .add_observer(on_start)
            .add_observer(on_skip)
            .add_observer(on_trigger_fade)
            .add_observer(on_trigger_keyframe)
            .add_systems(
                Update,
                (splash_dispatch, elapsed, fade, keyframe, apply_shape_alpha).chain(),
            );

        match app.get_sub_app_mut(RenderApp) {
            Some(render_app) => {
                render_app
                    .init_resource::<RenderReadiness>()
                    .add_systems(ExtractSchedule, sync_render_readiness)
                    .add_systems(
                        Render,
                        record_render_readiness.in_set(RenderSystems::Cleanup),
                    );
            }
            None => {
                app.insert_resource(RenderReadiness::READY);
            }
        }
    }
}

#[derive(Component, Clone, ExtractComponent)]
#[extract_app(RenderApp)]
struct SplashCamera;

#[derive(Resource, Clone, Copy, Default, Debug)]
struct RenderReadiness {
    // a window has presented at least one frame.
    // TODO: maybe find SplashCamera and check specifically whether
    // THAT window has presented at least once?
    window_presented: bool,
    // no render pipelines are queued or still compiling.
    pipelines_idle: bool,
    // the splash camera made it through extraction and has a view target.
    camera_rendered: bool,
}

impl RenderReadiness {
    const READY: Self = Self {
        window_presented: true,
        pipelines_idle: true,
        camera_rendered: true,
    };

    fn ready(&self) -> bool {
        self.window_presented && self.pipelines_idle && self.camera_rendered
    }
}

fn record_render_readiness(
    mut readiness: ResMut<RenderReadiness>,
    windows: Query<&ExtractedWindow>,
    pipeline_cache: Res<PipelineCache>,
    splash_views: Query<(), (With<SplashCamera>, With<ViewTarget>)>,
) {
    *readiness = RenderReadiness {
        window_presented: windows.iter().any(|window| !window.needs_initial_present),
        pipelines_idle: pipeline_cache.waiting_pipelines().next().is_none(),
        camera_rendered: !splash_views.is_empty(),
    };
}

fn sync_render_readiness(readiness: Res<RenderReadiness>, mut main_world: ResMut<MainWorld>) {
    main_world.insert_resource(*readiness);
}

/// Trigger this to play the splashscreen. Ignored if a splash is already running.
#[derive(Event, Default)]
pub struct StartBevySplashscreen;

/// Trigger this to end the splash immediately
#[derive(Event, Default)]
pub struct SkipBevySplashscreen;

/// Emitted after the splash finishes
#[derive(Event, Default)]
pub struct BevySplashscreenEnded;

#[derive(Component)]
struct SplashEntity;

#[derive(Reflect, Clone, Default)]
struct KeyFrame {
    start: Transform,
    end: Transform,
}

fn bird_keyframes() -> [KeyFrame; 3] {
    [
        KeyFrame {
            start: Transform {
                translation: Vec3::new(-203.0 + BIRD_SLIDE_OFFSET, 0.5, 2.0),
                rotation: Quat::from_euler(EulerRot::XYZ, -0.0, 0.0, -0.175),
                scale: Vec3::splat(3.8),
            },
            end: Transform {
                translation: Vec3::new(-203.0, 0.5, 2.0),
                rotation: Quat::from_euler(EulerRot::XYZ, -0.0, 0.0, -0.0),
                scale: Vec3::splat(3.8),
            },
        },
        KeyFrame {
            start: Transform {
                translation: Vec3::new(-200.0 + BIRD_SLIDE_OFFSET, 4.0, 1.0),
                rotation: Quat::from_euler(EulerRot::XYZ, -0.0, 0.0, -0.0),
                scale: Vec3::splat(3.3),
            },
            end: Transform {
                translation: Vec3::new(-175.7, 3.3, 1.0),
                rotation: Quat::from_euler(EulerRot::XYZ, -0.0, 0.0, -0.5),
                scale: Vec3::splat(3.3),
            },
        },
        KeyFrame {
            start: Transform {
                translation: Vec3::new(-235.0 + BIRD_SLIDE_OFFSET, 0.0, 0.0),
                rotation: Quat::from_euler(EulerRot::XYZ, -0.0, 0.0, -0.0),
                scale: Vec3::splat(3.0),
            },
            end: Transform {
                translation: Vec3::new(-156.8, -6.0, 0.0),
                rotation: Quat::from_euler(EulerRot::XYZ, -0.0, 0.0, -1.1),
                scale: Vec3::splat(3.0),
            },
        },
    ]
}

#[derive(Resource)]
struct Splash {
    started: bool,
    elapsed: f32,

    birds: [Entity; 3],
    built: Entity,
    with: Entity,
    bevy: Entity,
    cutoff: Entity,
    overlay: Entity,

    // time waited for render pipelines
    waited: f32,
}

/// How a [`Shape`]'s alpha is applied to its parts.
#[derive(Clone, Copy)]
enum ShapeBlend {
    /// Mix each part's color in from the background color.
    Background,
    /// Plain alpha blending, for single-part shapes drawn over other content.
    Alpha,
}

/// A group of [`ShapePart`] children faded together.
#[derive(Component)]
struct Shape {
    alpha: f32,
    blend: ShapeBlend,
}

impl Shape {
    fn part_color(&self, color: Color, background: Color) -> Color {
        match self.blend {
            // Mixed in linear space to match how the GPU alpha blends `ShapeBlend::Alpha`.
            ShapeBlend::Background => LinearRgba::from(background)
                .mix(&color.into(), self.alpha)
                .into(),
            ShapeBlend::Alpha => color.with_alpha(self.alpha),
        }
    }
}

/// One colored, tessellated piece of a [`Shape`].
#[derive(Component)]
struct ShapePart {
    color: Color,
}

/// Tessellates every filled/stroked path of an svg into meshes.
fn tessellate_svg(svg: &str, recolor: &[([u8; 3], Color)]) -> Vec<(Mesh, Color)> {
    let tree =
        usvg::Tree::from_str(svg, &usvg::Options::default()).expect("bundled svg failed to parse");
    let half = Vec2::new(tree.size().width(), tree.size().height()) / 2.0;

    let mut parts = Vec::new();
    collect_parts(tree.root(), half, recolor, &mut parts);
    parts
}

fn collect_parts(
    group: &usvg::Group,
    half: Vec2,
    recolor: &[([u8; 3], Color)],
    parts: &mut Vec<(Mesh, Color)>,
) {
    for node in group.children() {
        match node {
            usvg::Node::Group(group) => collect_parts(group, half, recolor, parts),
            usvg::Node::Path(path) if path.is_visible() => {
                let ts = path.abs_transform();
                let lyon_path = to_lyon_path(path.data(), ts, half);
                let fill = path.fill().map(|fill| {
                    let rule = match fill.rule() {
                        usvg::FillRule::NonZero => lyon_tessellation::FillRule::NonZero,
                        usvg::FillRule::EvenOdd => lyon_tessellation::FillRule::EvenOdd,
                    };
                    (
                        fill_mesh(&lyon_path, rule),
                        paint_color(fill.paint(), recolor),
                    )
                });
                let stroke = path.stroke().map(|stroke| {
                    let scale = (ts.sx * ts.sy - ts.kx * ts.ky).abs().sqrt();
                    (
                        stroke_mesh(&lyon_path, stroke, scale),
                        paint_color(stroke.paint(), recolor),
                    )
                });

                match path.paint_order() {
                    usvg::PaintOrder::FillAndStroke => parts.extend(fill.into_iter().chain(stroke)),
                    usvg::PaintOrder::StrokeAndFill => parts.extend(stroke.into_iter().chain(fill)),
                }
            }
            _ => {}
        }
    }
}

fn paint_color(paint: &usvg::Paint, recolor: &[([u8; 3], Color)]) -> Color {
    let usvg::Paint::Color(c) = paint else {
        panic!("bundled svgs only use solid colors");
    };
    let rgb = [c.red, c.green, c.blue];
    recolor
        .iter()
        .find(|(source, _)| *source == rgb)
        .map_or(Color::srgb_u8(c.red, c.green, c.blue), |(_, color)| *color)
}

fn to_lyon_path(data: &usvg::tiny_skia_path::Path, ts: usvg::Transform, half: Vec2) -> LyonPath {
    use usvg::tiny_skia_path::PathSegment;

    let map = |p: usvg::tiny_skia_path::Point| {
        let x = ts.sx * p.x + ts.kx * p.y + ts.tx;
        let y = ts.ky * p.x + ts.sy * p.y + ts.ty;
        point(x - half.x, half.y - y)
    };

    let mut builder = LyonPath::builder();
    let mut open = false;
    for segment in data.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                if open {
                    builder.end(false);
                }
                builder.begin(map(p));
                open = true;
            }
            PathSegment::LineTo(p) => {
                builder.line_to(map(p));
            }
            PathSegment::QuadTo(c, p) => {
                builder.quadratic_bezier_to(map(c), map(p));
            }
            PathSegment::CubicTo(c1, c2, p) => {
                builder.cubic_bezier_to(map(c1), map(c2), map(p));
            }
            PathSegment::Close => {
                builder.end(true);
                open = false;
            }
        }
    }
    if open {
        builder.end(false);
    }
    builder.build()
}

fn fill_mesh(path: &LyonPath, rule: lyon_tessellation::FillRule) -> Mesh {
    let mut buffers: VertexBuffers<[f32; 3], u32> = VertexBuffers::new();
    FillTessellator::new()
        .tessellate_path(
            path,
            &FillOptions::tolerance(TESSELLATION_TOLERANCE).with_fill_rule(rule),
            &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| {
                [v.position().x, v.position().y, 0.0]
            }),
        )
        .expect("bundled svg failed to tessellate");
    to_mesh(buffers)
}

fn stroke_mesh(path: &LyonPath, stroke: &usvg::Stroke, scale: f32) -> Mesh {
    let options = StrokeOptions::tolerance(TESSELLATION_TOLERANCE)
        .with_line_width(stroke.width().get() * scale)
        .with_miter_limit(stroke.miterlimit().get())
        .with_line_cap(match stroke.linecap() {
            usvg::LineCap::Butt => lyon_tessellation::LineCap::Butt,
            usvg::LineCap::Round => lyon_tessellation::LineCap::Round,
            usvg::LineCap::Square => lyon_tessellation::LineCap::Square,
        })
        .with_line_join(match stroke.linejoin() {
            usvg::LineJoin::Miter => lyon_tessellation::LineJoin::Miter,
            usvg::LineJoin::MiterClip => lyon_tessellation::LineJoin::MiterClip,
            usvg::LineJoin::Round => lyon_tessellation::LineJoin::Round,
            usvg::LineJoin::Bevel => lyon_tessellation::LineJoin::Bevel,
        });

    let mut buffers: VertexBuffers<[f32; 3], u32> = VertexBuffers::new();
    StrokeTessellator::new()
        .tessellate_path(
            path,
            &options,
            &mut BuffersBuilder::new(&mut buffers, |v: StrokeVertex| {
                [v.position().x, v.position().y, 0.0]
            }),
        )
        .expect("bundled svg failed to tessellate");
    to_mesh(buffers)
}

fn to_mesh(buffers: VertexBuffers<[f32; 3], u32>) -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, buffers.vertices)
    .with_inserted_indices(Indices::U32(buffers.indices))
}

/// Spawns a [`Shape`] with one mesh child per part.
fn spawn_shape(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    options: &BevySplashscreenOptions,
    parts: Vec<(Mesh, Color)>,
    shape: Shape,
    bundle: impl Bundle,
) -> Entity {
    let initial: Vec<Color> = parts
        .iter()
        .map(|(_, color)| shape.part_color(*color, options.background_color))
        .collect();
    commands
        .spawn((shape, bundle, Visibility::default()))
        .with_children(|parent| {
            for (index, (mesh, color)) in parts.into_iter().enumerate() {
                parent.spawn((
                    Mesh2d(meshes.add(mesh)),
                    MeshMaterial2d(materials.add(initial[index])),
                    Transform::from_xyz(0.0, 0.0, index as f32 * 0.001),
                    ShapePart { color },
                    RenderLayers::layer(options.splash_render_layer),
                ));
            }
        })
        .id()
}

fn on_start(
    _: On<StartBevySplashscreen>,
    existing: Option<Res<Splash>>,
    options: Res<BevySplashscreenOptions>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if existing.is_some() {
        return;
    }

    let bg = options.background_color;

    commands.spawn((
        Name::new("Splash camera"),
        SplashCamera,
        Camera2d,
        Camera {
            order: options.splash_camera_order,
            clear_color: ClearColorConfig::Custom(bg),
            ..default()
        },
        Msaa::Sample4,
        RenderLayers::layer(options.splash_render_layer),
        SplashEntity,
    ));

    // Everything is spawned visible but fully faded out.
    // This hypothetically ensures render pipelines are compiled *before* 
    // the meshes show up instead of during when they show up.
    let hidden = |blend| Shape { alpha: 0.0, blend };

    let built = spawn_shape(
        &mut commands,
        &mut meshes,
        &mut materials,
        &options,
        tessellate_svg(
            BUILT_SVG,
            &[(BUILT_TEXT_SOURCE_FILL, options.built_text_color)],
        ),
        hidden(ShapeBlend::Background),
        (
            Name::new("'Built' text"),
            Transform::from_xyz(55.0, 60.0, 0.0),
            Fade::default(),
            SplashEntity,
        ),
    );

    let with = spawn_shape(
        &mut commands,
        &mut meshes,
        &mut materials,
        &options,
        tessellate_svg(
            WITH_SVG,
            &[(WITH_TEXT_SOURCE_FILL, options.with_text_color)],
        ),
        hidden(ShapeBlend::Background),
        (
            Name::new("'With' text"),
            Transform::from_xyz(180.0, 60.0, 0.0),
            Fade::default(),
            SplashEntity,
        ),
    );

    let bevy = spawn_shape(
        &mut commands,
        &mut meshes,
        &mut materials,
        &options,
        tessellate_svg(
            BEVY_TEXT_SVG,
            &[(BEVY_TEXT_SOURCE_FILL, options.bevy_text_color)],
        ),
        hidden(ShapeBlend::Background),
        (
            Name::new("Bevy text"),
            Transform::from_xyz(90.0, -20.0, 0.0),
            Fade::default(),
            SplashEntity,
        ),
    );

    let unit_square = || vec![(Rectangle::new(1.0, 1.0).into(), bg)];

    let cutoff_rotation = Quat::from_euler(EulerRot::XYZ, 0.0, 0.0, -0.3);
    let cutoff_scale = Vec3::new(50.0, 100.0, 1.0);
    let cutoff_keyframe = KeyFrame {
        start: Transform {
            // Slightly behind the front bird (its parent) but in front of the others.
            translation: Vec3::new(-13.0, 5.8, -0.1),
            rotation: cutoff_rotation,
            scale: cutoff_scale,
        },
        end: Transform {
            translation: Vec3::new(-13.0, 5.8, -0.1),
            rotation: cutoff_rotation,
            scale: cutoff_scale,
        },
    };

    let cutoff = spawn_shape(
        &mut commands,
        &mut meshes,
        &mut materials,
        &options,
        unit_square(),
        hidden(ShapeBlend::Background),
        (
            Name::new("Tail/wing cutoff"),
            cutoff_keyframe.start,
            KeyframeInterp::from_keyframe(cutoff_keyframe),
        ),
    );

    let overlay = spawn_shape(
        &mut commands,
        &mut meshes,
        &mut materials,
        &options,
        unit_square(),
        hidden(ShapeBlend::Alpha),
        (
            Name::new("Fade-out overlay"),
            Transform::from_xyz(0.0, 0.0, 100.0).with_scale(Vec3::new(3000.0, 3000.0, 1.0)),
            Fade::default(),
            SplashEntity,
        ),
    );

    let mut birds = [Entity::PLACEHOLDER; 3];
    let keyframes = bird_keyframes();
    for index in 0..3 {
        let keyframe = keyframes[index].clone();

        birds[index] = spawn_shape(
            &mut commands,
            &mut meshes,
            &mut materials,
            &options,
            tessellate_svg(
                BIRD_SVG,
                &[
                    (BIRD_SOURCE_FILL, options.bird_colors[index]),
                    (BIRD_SOURCE_BG_FILL, bg),
                ],
            ),
            hidden(ShapeBlend::Background),
            (
                Name::new(BIRD_NAMES[index]),
                keyframe.start,
                KeyframeInterp {
                    start: false,
                    elapsed: 0.0,
                    keyframe,

                    translation: EaseFunction::BackInOut,
                    rotation: EaseFunction::BackInOut,
                    scale: EaseFunction::BackInOut,
                },
                SplashEntity,
            ),
        );
        if index == 0 {
            commands.entity(birds[index]).insert(Fade::default());
        }
    }

    commands.entity(cutoff).insert(ChildOf(birds[0]));

    commands.insert_resource(Splash {
        started: false,
        waited: 0.0,
        elapsed: 0.0,
        birds,
        built,
        with,
        bevy,
        cutoff,
        overlay,
    });
}

fn on_skip(
    _: On<SkipBevySplashscreen>,
    splash: Option<Res<Splash>>,
    splash_entities: Query<Entity, With<SplashEntity>>,
    mut commands: Commands,
) {
    if splash.is_none() {
        return;
    }
    end_splash(&mut commands, &splash_entities);
}

/// Despawns every splash entity
fn end_splash(commands: &mut Commands, splash_entities: &Query<Entity, With<SplashEntity>>) {
    for entity in splash_entities.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<Splash>();
    commands.trigger(BevySplashscreenEnded);
}

/// A scheduled action on the splash timeline, applied by [`splash_dispatch`]
/// once its time is crossed.
enum Step {
    Fade(Entity),
    Keyframe(Entity),
    Show(Entity),
}

#[derive(EntityEvent)]
struct TriggerFade {
    entity: Entity,
}

fn on_trigger_fade(on: On<TriggerFade>, mut fades: Query<&mut Fade>) {
    let Ok(mut fade) = fades.get_mut(on.entity) else {
        return;
    };
    fade.start = true;
    fade.elapsed = 0.0;
}

#[derive(EntityEvent)]
struct TriggerKeyFrame {
    entity: Entity,
}

fn on_trigger_keyframe(on: On<TriggerKeyFrame>, mut keyed: Query<&mut KeyframeInterp>) {
    let Ok(mut keyed) = keyed.get_mut(on.entity) else {
        return;
    };
    keyed.start = true;
    keyed.elapsed = 0.0;
}

#[derive(Component, Reflect)]
#[reflect(Component)]
struct Fade {
    start: bool,
    elapsed: f32,
    from: f32,
    to: f32,
}

impl Default for Fade {
    fn default() -> Self {
        Self {
            start: false,
            elapsed: 0.0,
            from: 0.0,
            to: 1.0,
        }
    }
}

#[derive(Component, Reflect)]
#[reflect(Component)]
struct KeyframeInterp {
    start: bool,
    elapsed: f32,
    keyframe: KeyFrame,

    translation: EaseFunction,
    rotation: EaseFunction,
    scale: EaseFunction,
}

impl KeyframeInterp {
    fn from_keyframe(key: KeyFrame) -> Self {
        Self {
            start: false,
            elapsed: 0.0,
            keyframe: key,
            translation: EaseFunction::CubicOut,
            rotation: EaseFunction::CubicOut,
            scale: EaseFunction::CubicOut,
        }
    }
}

fn elapsed(time: Res<Time>, mut fade: Query<&mut Fade>, mut key: Query<&mut KeyframeInterp>) {
    for mut fade in &mut fade {
        if fade.start {
            fade.elapsed += time.delta_secs();
        }
    }
    for mut key in &mut key {
        if key.start {
            key.elapsed += time.delta_secs();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn splash_dispatch(
    time: Res<Time>,
    real: Res<Time<Real>>,
    options: Res<BevySplashscreenOptions>,
    readiness: Res<RenderReadiness>,
    splash: Option<ResMut<Splash>>,
    splash_entities: Query<Entity, With<SplashEntity>>,
    fades: Query<&Fade>,
    mut shapes: Query<&mut Shape>,
    mut commands: Commands,
) {
    let Some(mut splash) = splash else {
        return;
    };

    if !splash.started {
        splash.waited += real.delta_secs();
        if !readiness.ready() {
            return;
        }

        info!("splash render ready after {:.0}ms", splash.waited * 1000.0);
        splash.started = true;
    }

    let last = splash.elapsed;
    splash.elapsed += time.delta_secs();

    let mut tasks = Vec::new();
    let mut c = 0.0;
    tasks.extend([
        (c, Step::Fade(splash.birds[0])),
        (c, Step::Fade(splash.built)),
        (c, Step::Fade(splash.with)),
        (c, Step::Fade(splash.bevy)),
    ]);

    c += options.fade_duration;
    tasks.extend([
        (c, Step::Show(splash.cutoff)),
        (c, Step::Show(splash.birds[1])),
        (c, Step::Show(splash.birds[2])),
        (c, Step::Keyframe(splash.birds[1])),
        (c, Step::Keyframe(splash.birds[2])),
        (c, Step::Keyframe(splash.cutoff)),
        (c, Step::Keyframe(splash.birds[0])),
    ]);

    c += options.keyframe_duration + options.hold_duration;
    tasks.push((c, Step::Fade(splash.overlay)));

    for (t, step) in tasks {
        if t >= last && t < splash.elapsed {
            match step {
                Step::Fade(entity) => commands.trigger(TriggerFade { entity }),
                Step::Keyframe(entity) => commands.trigger(TriggerKeyFrame { entity }),
                Step::Show(entity) => {
                    if let Ok(mut shape) = shapes.get_mut(entity) {
                        shape.alpha = 1.0;
                    }
                }
            }
        }
    }

    let overlay_covered = fades
        .get(splash.overlay)
        .is_ok_and(|fade| fade.start && fade.elapsed >= options.fade_duration);
    if overlay_covered {
        end_splash(&mut commands, &splash_entities);
    }
}

fn fade(mut fades: Query<(&Fade, &mut Shape)>, options: Res<BevySplashscreenOptions>) {
    for (fade, mut shape) in &mut fades {
        if !fade.start {
            continue;
        }

        let t = (fade.elapsed / options.fade_duration).clamp(0.0, 1.0);
        shape.alpha = fade.from + (fade.to - fade.from) * t;
    }
}

fn apply_shape_alpha(
    shapes: Query<(&Shape, &Children), Changed<Shape>>,
    parts: Query<(&ShapePart, &MeshMaterial2d<ColorMaterial>)>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    options: Res<BevySplashscreenOptions>,
) {
    for (shape, children) in &shapes {
        for (part, material) in parts.iter_many(children).flatten() {
            let Some(mut material) = materials.get_mut(&material.0) else {
                continue;
            };
            material.color = shape.part_color(part.color, options.background_color);
        }
    }
}

fn keyframe(
    mut keyed: Query<(&mut Transform, &KeyframeInterp)>,
    options: Res<BevySplashscreenOptions>,
) {
    for (mut transform, interp) in &mut keyed {
        if !interp.start {
            continue;
        }

        transform.translation = interp.keyframe.start.translation.lerp(
            interp.keyframe.end.translation,
            interp
                .translation
                .sample_clamped(interp.elapsed / options.keyframe_duration),
        );
        transform.rotation = interp.keyframe.start.rotation.slerp(
            interp.keyframe.end.rotation,
            interp
                .rotation
                .sample_clamped(interp.elapsed / options.keyframe_duration),
        );
        transform.scale = interp.keyframe.start.scale.lerp(
            interp.keyframe.end.scale,
            interp
                .scale
                .sample_clamped(interp.elapsed / options.keyframe_duration),
        );
    }
}
