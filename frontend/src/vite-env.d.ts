// Allow TypeScript to import CSS Module files
declare module '*.module.css' {
  const classes: { readonly [key: string]: string };
  export default classes;
}
