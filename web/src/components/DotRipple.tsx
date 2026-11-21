import {
	type Ref,
	type RefObject,
	useEffect,
	useImperativeHandle,
	useRef,
	useState,
} from "react";

const R = {
	duration: 4400,
	brightness: 50,
	trail: 400,
	trailBody: 100,
	front: 90,
	ease: 2.6,
	fade: 1.8,
	reach: 1.3,
	rearm: 400,
};

const GLYPHS = ["✚", "✖"];
const CELL = 11;
const RADIUS = 24;
const DEFAULT_SIZE = 500;
const HOLE_GAP = CELL;

type Hole = { x: number; y: number; w: number; h: number; radius: number };

/** Whether a glyph box centred at (x, y) lies fully inside the rounded rect. */
function insideRounded(x: number, y: number, w: number, h: number) {
	const half = CELL / 2;
	for (const [px, py] of [
		[x - half, y - half],
		[x + half, y - half],
		[x - half, y + half],
		[x + half, y + half],
	]) {
		if (px < 0 || py < 0 || px > w || py > h) return false;
		// Distance from the nearest corner-arc centre; only matters in the corners.
		const dx = px - Math.min(Math.max(px, RADIUS), w - RADIUS);
		const dy = py - Math.min(Math.max(py, RADIUS), h - RADIUS);
		if (dx * dx + dy * dy > RADIUS * RADIUS) return false;
	}
	return true;
}

/** Signed distance from (x, y) to the edge of a rounded rect; negative inside. */
function roundedDistance(x: number, y: number, hole: Hole) {
	const r = Math.min(hole.radius, hole.w / 2, hole.h / 2);
	const qx = Math.abs(x - (hole.x + hole.w / 2)) - (hole.w / 2 - r);
	const qy = Math.abs(y - (hole.y + hole.h / 2)) - (hole.h / 2 - r);
	return (
		Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) +
		Math.min(Math.max(qx, qy), 0) -
		r
	);
}

/**
 * One mask covering the whole element. Glyphs alternate along each row, odd rows
 * are offset half a cell, and any glyph the rounded outline would cut is omitted.
 */
function glyphMask(
	w: number,
	h: number,
	hole: Hole | null,
	edgeFade = 0,
	holeFade = 0,
) {
	const cols = Math.floor(w / CELL);
	const rows = Math.floor(h / CELL);
	// Centre the grid so the leftover margin is even on every side.
	const ox = (w - cols * CELL) / 2;
	const oy = (h - rows * CELL) / 2;
	let text = "";
	for (let row = 0; row < rows; row++) {
		for (let col = 0; col < cols; col++) {
			const x = ox + col * CELL + (row % 2 ? CELL : CELL / 2);
			const y = oy + row * CELL + CELL / 2;
			if (!insideRounded(x, y, w, h)) continue;
			const holeDist = hole
				? roundedDistance(x, y, hole)
				: Number.POSITIVE_INFINITY;
			if (holeDist < HOLE_GAP) continue;
			const edge = Math.min(x, y, w - x, h - y);
			const alpha = Math.min(
				edgeFade > 0 ? edge / edgeFade : 1,
				holeFade > 0 ? (holeDist - HOLE_GAP) / holeFade : 1,
				1,
			);
			if (alpha <= 0.05) continue;
			const opacity = alpha < 1 ? ` fill-opacity="${alpha.toFixed(2)}"` : "";
			text += `<text x="${x}" y="${y}"${opacity}>${GLYPHS[col % GLYPHS.length]}</text>`;
		}
	}
	const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}"><g font-family="ui-monospace,monospace" font-size="10" text-anchor="middle" dominant-baseline="central" fill="black">${text}</g></svg>`;
	return `url("data:image/svg+xml,${encodeURIComponent(svg)}")`;
}

type Origin = {
	x: number;
	y: number;
	radius: number;
	start: number;
	color?: string;
	brightness?: number;
	duration?: number;
};

export type DotRippleHandle = {
	/** Emit a ripple from a viewport point, optionally tinted and brighter. */
	pulse: (
		clientX: number,
		clientY: number,
		options?: { color?: string; brightness?: number; duration?: number },
	) => void;
};

function ripple(o: Origin, t: number) {
	const r = o.radius * R.reach * (1 - (1 - t) ** R.ease);
	const a = (o.brightness ?? R.brightness) * (1 - t) ** R.fade;
	const color = o.color ?? "var(--fg)";
	const c = (p: number) =>
		`color-mix(in srgb, ${color} ${p.toFixed(1)}%, transparent)`;
	const px = (d: number) => `${Math.max(0, r + d).toFixed(1)}px`;
	return `radial-gradient(circle at ${o.x}px ${o.y}px, transparent ${px(-R.trail)}, ${c((a * R.trailBody) / 100)} ${px(-R.trail / 2)}, ${c(a)} ${px(0)}, transparent ${px(R.front)})`;
}

type DotRippleProps = {
	/** Element laid over the grid; glyphs behind it are left out of the mask. */
	holeRef?: RefObject<HTMLElement | null>;
	/** Distance in px over which glyphs fade out towards the element's edges. */
	edgeFade?: number;
	/** Distance in px over which glyphs fade out towards the hole. */
	holeFade?: number;
	/** Imperative handle for emitting ripples, e.g. on submit. */
	handle?: Ref<DotRippleHandle>;
};

/** Masked dot grid that emits a ripple from where the pointer enters it. */
export function DotRipple({
	holeRef,
	edgeFade = 0,
	holeFade = 0,
	handle,
}: DotRippleProps) {
	const ref = useRef<HTMLDivElement>(null);
	const spawn = useRef<
		(clientX: number, clientY: number, extra?: Partial<Origin>) => void
	>(() => {});

	useImperativeHandle(
		handle,
		() => ({
			pulse: (clientX, clientY, options) =>
				spawn.current(clientX, clientY, options),
		}),
		[],
	);
	const [size, setSize] = useState({ w: DEFAULT_SIZE, h: DEFAULT_SIZE });
	const [hole, setHole] = useState<Hole | null>(null);

	// Rebuild the mask whenever the grid or the element over it resizes.
	useEffect(() => {
		const el = ref.current;
		if (!el) return;
		const holeEl = holeRef?.current ?? null;
		const measure = () => {
			const b = el.getBoundingClientRect();
			const w = Math.round(b.width);
			const h = Math.round(b.height);
			setSize((prev) => (prev.w === w && prev.h === h ? prev : { w, h }));
			if (!holeEl) return;
			const c = holeEl.getBoundingClientRect();
			const next = {
				x: Math.round(c.left - b.left),
				y: Math.round(c.top - b.top),
				w: Math.round(c.width),
				h: Math.round(c.height),
				radius:
					Number.parseFloat(getComputedStyle(holeEl).borderTopLeftRadius) || 0,
			};
			setHole((prev) =>
				prev &&
				prev.x === next.x &&
				prev.y === next.y &&
				prev.w === next.w &&
				prev.h === next.h &&
				prev.radius === next.radius
					? prev
					: next,
			);
		};
		const observer = new ResizeObserver(measure);
		observer.observe(el);
		if (holeEl) observer.observe(holeEl);
		return () => observer.disconnect();
	}, [holeRef]);

	useEffect(() => {
		const el = ref.current;
		if (!el || matchMedia("(prefers-reduced-motion: reduce)").matches) return;

		// Each entry adds its own ripple; overlapping ripples are layered gradients.
		let origins: Origin[] = [];
		let frame = 0;
		let lastLeave = Number.NEGATIVE_INFINITY;

		const tick = (now: number) => {
			origins = origins.filter(
				(o) => now - o.start < (o.duration ?? R.duration),
			);
			if (origins.length === 0) {
				frame = 0;
				el.style.removeProperty("--dot-ripple");
				return;
			}
			el.style.setProperty(
				"--dot-ripple",
				origins
					.map((o) => ripple(o, (now - o.start) / (o.duration ?? R.duration)))
					.join(", "),
			);
			frame = requestAnimationFrame(tick);
		};

		const add = (clientX: number, clientY: number, extra?: Partial<Origin>) => {
			const b = el.getBoundingClientRect();
			const x = clientX - b.left;
			const y = clientY - b.top;
			origins.push({
				x,
				y,
				radius: Math.hypot(Math.max(x, b.width - x), Math.max(y, b.height - y)),
				start: performance.now(),
				...extra,
			});
			if (frame === 0) frame = requestAnimationFrame(tick);
		};
		spawn.current = add;

		const enter = (e: PointerEvent) => {
			if (performance.now() - lastLeave < R.rearm) return;
			add(e.clientX, e.clientY);
		};
		const leave = () => {
			lastLeave = performance.now();
		};

		el.addEventListener("pointerenter", enter);
		el.addEventListener("pointerleave", leave);
		return () => {
			el.removeEventListener("pointerenter", enter);
			el.removeEventListener("pointerleave", leave);
			cancelAnimationFrame(frame);
			spawn.current = () => {};
		};
	}, []);

	return (
		<div
			ref={ref}
			className="dot-ripple"
			aria-hidden="true"
			style={{
				maskImage: glyphMask(size.w, size.h, hole, edgeFade, holeFade),
			}}
		/>
	);
}
