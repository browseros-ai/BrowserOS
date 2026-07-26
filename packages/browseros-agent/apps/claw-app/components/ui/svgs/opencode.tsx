import type { SVGProps } from "react";

const Opencode = (props: SVGProps<SVGSVGElement>) => (
  <svg {...props} viewBox="0 0 512 512" fill="none">
    <rect width="512" height="512" fill="#FDFCFC" />
    <path d="M320 224V352H192V224H320Z" fill="#E6E5E6" />
    <path
      fillRule="evenodd"
      clipRule="evenodd"
      d="M384 416H128V96H384V416ZM320 160H192V352H320V160Z"
      fill="#17181C"
    />
  </svg>
);

export { Opencode };
