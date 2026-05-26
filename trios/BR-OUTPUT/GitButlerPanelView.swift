import SwiftUI

struct GitButlerPanelView: View {
    @StateObject private var vm = GitButlerViewModel()
    @State private var newBranchName = ""
    @State private var commitMessage = ""
    @State private var selectedBranch: VirtualBranch?

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().overlay(Color.grokBorder)
            branchList
        }
        .background(Color.clear)
    }

    private var header: some View {
        HStack(spacing: 8) {
            Text("Virtual Branches")
                .font(.system(size: 13, weight: .semibold))
                .foregroundColor(.grokText)
            Spacer()
            if vm.isApplying {
                ProgressView()
                    .scaleEffect(0.6)
            }
            Button(action: { vm.loadBranches() }) {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 11))
                    .foregroundColor(.grokMuted)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private var branchList: some View {
        VStack(spacing: 0) {
            if vm.branches.isEmpty {
                VStack(spacing: 8) {
                    Spacer()
                    Image(systemName: "arrow.triangle.branch")
                        .font(.system(size: 32))
                        .foregroundColor(.grokDim)
                    Text("No branches")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundColor(.grokText)
                    Text("Create a new branch to get started")
                        .font(.system(size: 11))
                        .foregroundColor(.grokMuted)
                    Spacer()
                }
            } else {
                List(vm.branches) { branch in
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 8) {
                        Circle()
                            .fill(branch.isApplied ? Color.green.opacity(0.8) : Color.grokDim)
                            .frame(width: 6, height: 6)

                        Text(branch.name)
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundColor(.grokText)

                        Spacer()

                        if branch.isConflicted {
                            Text("Conflict")
                                .font(.system(size: 9))
                                .padding(.horizontal, 5)
                                .padding(.vertical, 1)
                                .background(Color.red.opacity(0.2))
                                .cornerRadius(4)
                                .foregroundColor(.grokText)
                        }

                        Text("\(branch.files)f \(branch.commitCount)c")
                            .font(.system(size: 9))
                            .foregroundColor(.grokMuted)
                    }

                    if let upstream = branch.upstream {
                        Text(upstream)
                            .font(.system(size: 9))
                            .foregroundColor(.grokDim)
                    }

                    HStack(spacing: 6) {
                        if !branch.isApplied {
                            Button("Switch") {
                                vm.switchBranch(branch)
                            }
                            .font(.system(size: 10))
                            .foregroundColor(.grokAccent)
                            .buttonStyle(.plain)
                        }

                        Button("Push") {
                            vm.pushBranch(branch)
                        }
                        .font(.system(size: 10))
                        .foregroundColor(.grokMuted)
                        .buttonStyle(.plain)

                        Button("Commit") {
                            selectedBranch = branch
                        }
                        .font(.system(size: 10))
                        .foregroundColor(.grokMuted)
                        .buttonStyle(.plain)

                        if !branch.isApplied {
                            Button("Delete") {
                                vm.deleteBranch(branch)
                            }
                            .font(.system(size: 10))
                            .foregroundColor(.red.opacity(0.6))
                            .buttonStyle(.plain)
                        }
                    }
                }
                .padding(.vertical, 4)
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            }

            if !vm.consoleOutput.isEmpty {
                VStack(spacing: 0) {
                    Divider().overlay(Color.grokBorder)
                    Text(vm.consoleOutput)
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundColor(.grokMuted)
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color.grokSurface)
                }
            }

            createBranchBar
        }
        .sheet(item: $selectedBranch) { branch in
            commitSheet(for: branch)
        }
    }

    private var createBranchBar: some View {
        HStack(spacing: 8) {
            TextField("New branch...", text: $newBranchName)
                .font(.system(size: 12))
                .foregroundColor(.grokText)
                .textFieldStyle(PlainTextFieldStyle())

            Button(action: {
                guard !newBranchName.isEmpty else { return }
                vm.createBranch(name: newBranchName)
                newBranchName = ""
            }) {
                Image(systemName: "plus")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundColor(.grokAccent)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Color.grokElevated.opacity(0.4))
    }

    private func commitSheet(for branch: VirtualBranch) -> some View {
        VStack(spacing: 12) {
            Text("Commit \(branch.name)")
                .font(.system(size: 13, weight: .semibold))
                .foregroundColor(.grokText)

            TextField("Message", text: $commitMessage)
                .font(.system(size: 12))
                .textFieldStyle(PlainTextFieldStyle())
                .padding(8)
                .background(Color.grokElevated)
                .cornerRadius(6)

            HStack(spacing: 12) {
                Button("Cancel") {
                    selectedBranch = nil
                    commitMessage = ""
                }
                .foregroundColor(.grokMuted)
                .buttonStyle(.plain)

                Button("Commit") {
                    guard !commitMessage.isEmpty else { return }
                    vm.commitBranch(branch, message: commitMessage)
                    selectedBranch = nil
                    commitMessage = ""
                }
                .foregroundColor(.grokAccent)
                .buttonStyle(.plain)
            }
        }
        .padding(16)
        .frame(width: 300)
        .background(Color.grokBackground)
    }
}
