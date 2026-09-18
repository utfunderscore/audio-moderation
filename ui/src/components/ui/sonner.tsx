"use client"

import type { CSSProperties } from "react"
import {
  AlertOctagon,
  AlertTriangle,
  CheckCircle,
  InfoCircle,
  Loading01,
} from "@untitledui/icons"
import { Toaster as Sonner, type ToasterProps } from "sonner"

import { useTheme } from "@/components/theme-provider"

const Toaster = ({ ...props }: ToasterProps) => {
  const { theme } = useTheme()

  return (
    <Sonner
      theme={theme as ToasterProps["theme"]}
      className="toaster group"
      icons={{
        success: <CheckCircle className="size-4" />,
        info: <InfoCircle className="size-4" />,
        warning: <AlertTriangle className="size-4" />,
        error: <AlertOctagon className="size-4" />,
        loading: (
          <Loading01 className="size-4 animate-spin motion-reduce:animate-none" />
        ),
      }}
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
          "--border-radius": "var(--radius)",
        } as CSSProperties
      }
      toastOptions={{
        classNames: {
          toast: "cn-toast",
        },
      }}
      {...props}
    />
  )
}

export { Toaster }
