const paths: Record<string, string> = {
  play: "M7 4.5v15l12-7.5z",
  pause: "M7 4h3.5v16H7zM13.5 4H17v16h-3.5z",
  star: "M12 3.5l2.6 5.4 5.9.8-4.3 4.1 1 5.9L12 16.9l-5.2 2.8 1-5.9-4.3-4.1 5.9-.8z",
  folder: "M3.5 6.5a1.5 1.5 0 0 1 1.5-1.5h4.2l2 2.2H19a1.5 1.5 0 0 1 1.5 1.5v8.8A1.5 1.5 0 0 1 19 19H5a1.5 1.5 0 0 1-1.5-1.5z",
  copy: "M8.5 8.5h10v11h-10zM5.5 15.5v-11h10",
  trash: "M4.5 7h15M9.5 7V4.5h5V7M6.5 7l1 12.5h9l1-12.5M10 10.5v6M14 10.5v6",
  scissors: "M8.5 8.5L19 18M8.5 15.5L19 6M6.5 9.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5zM6.5 19.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z",
  send: "M4 12l16-7.5L13 20l-2.2-6.3z M10.8 13.7L20 4.5",
  search: "M10.5 17a6.5 6.5 0 1 0 0-13 6.5 6.5 0 0 0 0 13zM15.5 15.5L20 20",
  gear: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 13.5l1.6 1.2-1.8 3.1-1.9-.7a7.5 7.5 0 0 1-2.1 1.2l-.3 2H11l-.3-2a7.5 7.5 0 0 1-2.1-1.2l-1.9.7-1.8-3.1 1.6-1.2a7.6 7.6 0 0 1 0-2.6L4.9 9.7l1.8-3.1 1.9.7A7.5 7.5 0 0 1 10.7 6l.3-2h3.6l.3 2a7.5 7.5 0 0 1 2.1 1.2l1.9-.7 1.8 3.1-1.6 1.2a7.6 7.6 0 0 1 .3 2.7z",
  grid: "M4.5 4.5h6v6h-6zM13.5 4.5h6v6h-6zM4.5 13.5h6v6h-6zM13.5 13.5h6v6h-6z",
  film: "M4.5 5.5h15v13h-15zM8 5.5v13M16 5.5v13M4.5 9h3.5M4.5 15h3.5M16 9h3.5M16 15h3.5",
  back: "M14.5 5.5L8 12l6.5 6.5",
  export: "M12 15V4.5M7.5 9L12 4.5 16.5 9M5 14v5.5h14V14",
  volume: "M4.5 9.5h3.5L12.5 5.5v13l-4.5-4H4.5zM16 9a4 4 0 0 1 0 6M18.5 6.5a7.5 7.5 0 0 1 0 11",
  mute: "M4.5 9.5h3.5L12.5 5.5v13l-4.5-4H4.5zM16.5 9.5l5 5M21.5 9.5l-5 5",
  pin: "M12 4.5v15",
};

export function Icon({ name, size = 18, fill = false }: { name: keyof typeof paths | string; size?: number; fill?: boolean }) {
  const solid = fill || name === "play" || name === "pause";
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      aria-hidden="true"
      fill={solid ? "currentColor" : "none"}
      stroke={solid && name !== "star" ? "none" : "currentColor"}
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d={paths[name]} />
    </svg>
  );
}
