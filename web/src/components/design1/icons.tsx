import type { SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement>;

function Stroke({ children, ...props }: IconProps) {
	return (
		<svg
			viewBox="0 0 24 24"
			fill="none"
			stroke="currentColor"
			strokeWidth="2"
			strokeLinecap="round"
			strokeLinejoin="round"
			aria-hidden="true"
			{...props}
		>
			{children}
		</svg>
	);
}

export function GitHubIcon(props: IconProps) {
	return (
		<svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true" {...props}>
			<path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8Z" />
		</svg>
	);
}

export function UploadIcon(props: IconProps) {
	return (
		<Stroke {...props}>
			<path d="M12 15V3" />
			<path d="m7 8 5-5 5 5" />
			<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
		</Stroke>
	);
}

export function CheckIcon(props: IconProps) {
	return (
		<Stroke strokeWidth="3" {...props}>
			<path d="M20 6 9 17l-5-5" />
		</Stroke>
	);
}

export function CrossIcon(props: IconProps) {
	return (
		<Stroke strokeWidth="3" {...props}>
			<path d="M18 6 6 18" />
			<path d="m6 6 12 12" />
		</Stroke>
	);
}

export function ArrowRightIcon(props: IconProps) {
	return (
		<Stroke {...props}>
			<path d="M5 12h14" />
			<path d="m12 5 7 7-7 7" />
		</Stroke>
	);
}

export function RestartIcon(props: IconProps) {
	return (
		<Stroke {...props}>
			<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
			<path d="M3 3v5h5" />
		</Stroke>
	);
}

export function PlayIcon(props: IconProps) {
	return (
		<Stroke strokeWidth="2.25" {...props}>
			<path d="M7 4.5v15a1 1 0 0 0 1.5.86l12.5-7.5a1 1 0 0 0 0-1.72L8.5 3.64A1 1 0 0 0 7 4.5Z" />
		</Stroke>
	);
}

export function PauseIcon(props: IconProps) {
	return (
		<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" {...props}>
			<rect x="6" y="4.5" width="4" height="15" rx="1.2" />
			<rect x="14" y="4.5" width="4" height="15" rx="1.2" />
		</svg>
	);
}

export function VolumeIcon(props: IconProps) {
	return (
		<Stroke {...props}>
			<path d="M11 5 6 9H3v6h3l5 4V5Z" />
			<path d="M15.5 9.5a3.5 3.5 0 0 1 0 5" />
			<path d="M18.5 7a7 7 0 0 1 0 10" />
		</Stroke>
	);
}

export function MutedIcon(props: IconProps) {
	return (
		<Stroke {...props}>
			<path d="M11 5 6 9H3v6h3l5 4V5Z" />
			<path d="m16 9 5 5" />
			<path d="m21 9-5 5" />
		</Stroke>
	);
}
