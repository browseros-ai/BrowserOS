package ota

import "fmt"

func runListPlatforms() error {
	fmt.Println("Available Server Platforms:")
	fmt.Println("--------------------------------------------------")
	for _, platform := range serverPlatforms {
		fmt.Printf("  %-15s %-10s %s\n", platform.Name, platform.OS, platform.Arch)
	}
	fmt.Println("--------------------------------------------------")
	return nil
}
