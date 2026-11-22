import { useEffect, useRef } from "react";

// Reduced retro palette.
const STAR_COLORS = [
	"#FFFFFF",
	"#FFFFAA",
	"#AAAAFF",
	"#FFAAAA",
	"#AAFFAA",
	"#FFAAFF",
	"#AAFFFF",
] as const;

const STAR_DENSITY = 0.00004; // stars per free CSS px²
const TWINKLE_PROBABILITY = 0.7;
const MIN_TWINKLE_SECONDS = 2;
const MAX_TWINKLE_SECONDS = 4;
const PIXEL = 5;
/** Clearance kept between a star and any avoided element. */
const AVOID_MARGIN = PIXEL * 2;
/** Random placements tried before giving up on a star. */
const PLACEMENT_TRIES = 24;
const REGENERATE_MS = 5000;
const REGENERATE_FRACTION = 0.15;
const SHOOTING_PIXEL = 2;
const TRAIL_COLOR = "rgb(180 242 255)";
const TRAIL_FADE = 0.1;
const FPS = 16; // deliberately choppy for the retro feel
const FRAME_MS = 1000 / FPS;
const BOUNDS_MARGIN = 30;
/** Pixels of the 4×2 shooting-star head, with two corners knocked out. */
const HEAD = [
	[0, 0],
	[1, 0],
	[2, 0],
	[1, 1],
	[2, 1],
	[3, 1],
] as const;

type Star = {
	x: number;
	y: number;
	color: string;
	bright: number;
	dim: number;
	/** Seconds per half-cycle; 0 means the star does not twinkle. */
	period: number;
	timer: number;
	fadingOut: boolean;
};

type TrailPoint = { x: number; y: number; opacity: number };

type ShootingStar = {
	x: number;
	y: number;
	dx: number;
	dy: number;
	cos: number;
	sin: number;
	trail: TrailPoint[];
};

/** An avoided area in canvas coordinates, already grown by the margin. */
type Box = { left: number; top: number; right: number; bottom: number };

/** Where stars may go: the canvas size minus the avoided boxes. */
type Field = { width: number; height: number; avoid: Box[] };

const random = (min: number, max: number) => min + Math.random() * (max - min);

function isBlocked(x: number, y: number, avoid: Box[]) {
	for (const b of avoid) {
		if (x + PIXEL > b.left && x < b.right && y + PIXEL > b.top && y < b.bottom)
			return true;
	}
	return false;
}

/** A new star on a free grid cell, or null if none was found quickly. */
function makeStar({ width, height, avoid }: Field): Star | null {
	for (let i = 0; i < PLACEMENT_TRIES; i++) {
		const x = Math.floor(Math.random() * (width / PIXEL)) * PIXEL;
		const y = Math.floor(Math.random() * (height / PIXEL)) * PIXEL;
		if (isBlocked(x, y, avoid)) continue;
		const bright = random(0.5, 1);
		return {
			x,
			y,
			color:
				STAR_COLORS[Math.floor(Math.random() * STAR_COLORS.length)] ?? "#FFF",
			bright,
			dim: bright * 0.3,
			period:
				Math.random() < TWINKLE_PROBABILITY
					? random(MIN_TWINKLE_SECONDS, MAX_TWINKLE_SECONDS)
					: 0,
			timer: 0,
			fadingOut: true,
		};
	}
	return null;
}

/** Star count for the uncovered area, so density matches what is visible. */
function starCount({ width, height, avoid }: Field) {
	let free = width * height;
	for (const b of avoid) {
		const w = Math.min(b.right, width) - Math.max(b.left, 0);
		const h = Math.min(b.bottom, height) - Math.max(b.top, 0);
		if (w > 0 && h > 0) free -= w * h;
	}
	return Math.floor(Math.max(0, free) * STAR_DENSITY);
}

function makeShootingStar(width: number): ShootingStar {
	// 45°–135°: down-right through straight down to down-left.
	const angle = (random(45, 135) * Math.PI) / 180;
	const speed = random(8, 13);
	const cos = Math.cos(angle);
	const sin = Math.sin(angle);
	return {
		x: Math.random() * width,
		y: 0,
		dx: speed * cos,
		dy: speed * sin,
		cos,
		sin,
		trail: [],
	};
}

/** Two-level twinkle: bright, then dim, flipping direction every period. */
function starOpacity(star: Star) {
	if (star.period === 0) return star.bright;
	const firstHalf = star.timer / star.period < 0.5;
	return firstHalf === star.fadingOut ? star.bright : star.dim;
}

type PixelStarsProps = {
	/**
	 * Selector for elements stars must not be placed behind. Matching elements
	 * are tracked as they resize, and stars they grow over are moved.
	 */
	avoid?: string;
};

/**
 * Canvas of twinkling pixel stars with occasional shooting stars. It fills its
 * positioned parent and scrolls with it, so avoided areas stay put.
 */
export function PixelStars({ avoid }: PixelStarsProps) {
	const ref = useRef<HTMLCanvasElement>(null);

	useEffect(() => {
		const canvas = ref.current;
		const ctx = canvas?.getContext("2d", { alpha: true });
		if (!canvas || !ctx) return;

		const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");
		let field: Field = { width: 0, height: 0, avoid: [] };
		let stars: Star[] = [];
		let shooting: ShootingStar[] = [];
		let frame = 0;
		let last = 0;
		let spawnTimer = 0;

		const drawStars = () => {
			for (const star of stars) {
				ctx.fillStyle = star.color;
				ctx.globalAlpha = starOpacity(star);
				ctx.fillRect(star.x, star.y, PIXEL, PIXEL);
			}
			ctx.globalAlpha = 1;
		};

		const drawShooting = (dpr: number) => {
			for (const s of shooting) {
				// Rotate about each point by composing the transform directly instead
				// of save/translate/rotate/restore per pixel.
				ctx.fillStyle = TRAIL_COLOR;
				for (const p of s.trail) {
					ctx.globalAlpha = p.opacity;
					ctx.setTransform(
						dpr * s.cos,
						dpr * s.sin,
						-dpr * s.sin,
						dpr * s.cos,
						dpr * p.x,
						dpr * p.y,
					);
					ctx.fillRect(0, 0, SHOOTING_PIXEL, SHOOTING_PIXEL);
				}
				ctx.globalAlpha = 1;
				ctx.fillStyle = "#FFFFFF";
				ctx.setTransform(
					dpr * s.cos,
					dpr * s.sin,
					-dpr * s.sin,
					dpr * s.cos,
					dpr * s.x,
					dpr * s.y,
				);
				for (const [px, py] of HEAD) {
					ctx.fillRect(
						px * SHOOTING_PIXEL,
						py * SHOOTING_PIXEL,
						SHOOTING_PIXEL,
						SHOOTING_PIXEL,
					);
				}
			}
			ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
		};

		const render = () => {
			const dpr = window.devicePixelRatio || 1;
			ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
			ctx.clearRect(0, 0, field.width, field.height);
			drawStars();
			if (shooting.length) drawShooting(dpr);
		};

		const step = () => {
			for (const star of stars) {
				if (star.period === 0) continue;
				star.timer += 1 / FPS;
				if (star.timer >= star.period) {
					star.timer = 0;
					star.fadingOut = !star.fadingOut;
				}
			}
			// Mutate in place and compact, so a frame allocates nothing but new
			// trail points.
			let kept = 0;
			for (const s of shooting) {
				s.trail.push({ x: s.x, y: s.y, opacity: 1 });
				let live = 0;
				for (const p of s.trail) {
					p.opacity -= TRAIL_FADE;
					if (p.opacity > 0) s.trail[live++] = p;
				}
				s.trail.length = live;
				s.x += s.dx;
				s.y += s.dy;
				if (
					s.x >= -BOUNDS_MARGIN &&
					s.x <= field.width + BOUNDS_MARGIN &&
					s.y <= field.height + BOUNDS_MARGIN
				) {
					shooting[kept++] = s;
				}
			}
			shooting.length = kept;
		};

		const tick = (now: number) => {
			frame = requestAnimationFrame(tick);
			const elapsed = now - last;
			if (elapsed < FRAME_MS) return;
			// Keep the cadence aligned to the frame grid instead of drifting.
			last = now - (elapsed % FRAME_MS);
			step();
			render();
		};

		const scheduleShootingStar = () => {
			spawnTimer = window.setTimeout(
				() => {
					// Background tabs keep timers running; don't queue a burst for later.
					if (!document.hidden) shooting.push(makeShootingStar(field.width));
					scheduleShootingStar();
				},
				random(2000, 6000),
			);
		};

		const regenerate = () => {
			if (document.hidden || stars.length === 0) return;
			const count = Math.max(1, Math.floor(stars.length * REGENERATE_FRACTION));
			for (let i = 0; i < count; i++) {
				const star = makeStar(field);
				if (star) stars[Math.floor(Math.random() * stars.length)] = star;
			}
			if (reducedMotion.matches) render();
		};

		const avoidedElements = () =>
			avoid ? Array.from(document.querySelectorAll(avoid)) : [];

		const measureAvoid = (): Box[] => {
			const origin = canvas.getBoundingClientRect();
			return avoidedElements().map((el) => {
				const r = el.getBoundingClientRect();
				return {
					left: r.left - origin.left - AVOID_MARGIN,
					top: r.top - origin.top - AVOID_MARGIN,
					right: r.right - origin.left + AVOID_MARGIN,
					bottom: r.bottom - origin.top + AVOID_MARGIN,
				};
			});
		};

		const layout = () => {
			const dpr = window.devicePixelRatio || 1;
			const width = canvas.clientWidth;
			const height = canvas.clientHeight;
			const resized = width !== field.width || height !== field.height;
			field = { width, height, avoid: measureAvoid() };

			if (resized) {
				// Back the canvas at device resolution so 5px stars stay crisp on HiDPI.
				canvas.width = Math.round(width * dpr);
				canvas.height = Math.round(height * dpr);
			}
			// Keep stars that are still visible so layout changes (e.g. the demo
			// card growing) don't reshuffle the sky; then top up or trim to the
			// density for the new free area.
			const target = starCount(field);
			stars = stars.filter(
				(s) =>
					s.x + PIXEL <= width &&
					s.y + PIXEL <= height &&
					!isBlocked(s.x, s.y, field.avoid),
			);
			for (let misses = 0; stars.length < target && misses < 8; ) {
				const star = makeStar(field);
				if (star) stars.push(star);
				else misses++;
			}
			stars.length = Math.min(stars.length, target);
			render();
		};

		let layoutFrame = 0;
		const scheduleLayout = () => {
			cancelAnimationFrame(layoutFrame);
			layoutFrame = requestAnimationFrame(layout);
		};

		// The canvas box and every avoided element can change size independently
		// (e.g. the demo card grows when results arrive). Re-observing is a no-op
		// for elements already tracked and picks up newly mounted ones.
		const observer = new ResizeObserver(() => {
			for (const el of avoidedElements()) observer.observe(el);
			scheduleLayout();
		});
		observer.observe(canvas);
		for (const el of avoidedElements()) observer.observe(el);
		// Shifts in position without a size change (e.g. content above growing)
		// are caught by the canvas or a sibling resizing; window resizes cover
		// reflowed text.
		window.addEventListener("resize", scheduleLayout);

		const start = () => {
			stop();
			if (reducedMotion.matches) {
				shooting = [];
				render();
				return;
			}
			frame = requestAnimationFrame(tick);
			scheduleShootingStar();
		};

		function stop() {
			cancelAnimationFrame(frame);
			clearTimeout(spawnTimer);
		}

		layout();
		start();
		const regenTimer = window.setInterval(regenerate, REGENERATE_MS);
		reducedMotion.addEventListener("change", start);

		return () => {
			stop();
			cancelAnimationFrame(layoutFrame);
			clearInterval(regenTimer);
			observer.disconnect();
			window.removeEventListener("resize", scheduleLayout);
			reducedMotion.removeEventListener("change", start);
		};
	}, [avoid]);

	return <canvas ref={ref} className="pixel-stars" />;
}
