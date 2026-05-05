import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import App from "./App";

describe("App", () => {
  it("renders the sidebar with navigation links", () => {
    render(<App />);
    expect(screen.getByText("Beachead")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Personas" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Agents" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Sessions" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Policies" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Help" })).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "System Settings" }),
    ).toBeInTheDocument();
  });

  it("navigates to personas page by default", () => {
    render(<App />);
    expect(screen.getByText("Manage your AI agent personas here.")).toBeInTheDocument();
  });
});
