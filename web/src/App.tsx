import * as React from "react";

export const App = () => {
  const [state, setState] = React.useState(1);
  const handleClick = React.useCallback(() => {
    setState((x) => x + 1);
  }, []);
  return (
    <>
      <h1>Hello, React! number = {state}</h1>
      <button type="button" onClick={handleClick}>
        Click me
      </button>
    </>
  );
};
