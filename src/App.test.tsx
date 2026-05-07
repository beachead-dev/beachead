import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import App from "./App";

describe("App", () => {
  it("renders the sidebar with navigation links", () => {
    render(<App />);
    expect(screen.getAllByAltText("Beachead").length).toBeGreaterThan(0);
    expect(screen.getByRole("link", { name: "Personas" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Agents" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Sessions" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Policies" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Help and documentation" })).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "System Settings" }),
    ).toBeInTheDocument();
  });

  it("navigates to sessions page by default", () => {
    render(<App />);
    expect(screen.getByRole("heading", { name: "Sessions" })).toBeInTheDocument();
  });
});
