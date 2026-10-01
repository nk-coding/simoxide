package de.uka.ipd.sdq.scheduler.resources.active;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

import de.uka.ipd.sdq.scheduler.ISchedulableProcess;

public class ResourceTableManager implements IResourceTableManager {
    // REFSIM: was ConcurrentHashMap (identity-hash order). waitForProcesses() re-activates the remaining
    // processes at simulation end in iteration order -> use insertion order (see waitForProcesses).
    private final Map<ISchedulableProcess, AbstractActiveResource> currentResourceTable = java.util.Collections
        .synchronizedMap(new java.util.LinkedHashMap<>());

    @Override
    public AbstractActiveResource getLastResource(ISchedulableProcess process) {
        return currentResourceTable.get(process);
    }

    @Override
    public void setLastResource(ISchedulableProcess process, AbstractActiveResource resource) {
        if (!currentResourceTable.containsKey(process)) {
            process.addTerminatedObserver(resource);
        }
        currentResourceTable.put(process, resource);
    }

    public void notifyTerminated(ISchedulableProcess simProcess) {
        currentResourceTable.remove(simProcess);
    }
    
    @Override
    public void waitForProcesses() {
        // Activate all waiting processes to yield process completion
        // Synchronization with process() avoids that processes are added after
        // the activation.
        // REFSIM: iterate over snapshots in insertion order (activated processes remove themselves via
        // notifyTerminated(), and may register new ones); repeat until no unvisited process is left.
        final java.util.Set<ISchedulableProcess> visited = java.util.Collections
            .newSetFromMap(new java.util.IdentityHashMap<>());
        while (true) {
            final java.util.List<ISchedulableProcess> snapshot;
            synchronized (currentResourceTable) {
                snapshot = new java.util.ArrayList<>(currentResourceTable.keySet());
            }
            snapshot.removeIf(visited::contains);
            if (snapshot.isEmpty()) {
                break;
            }
            for (ISchedulableProcess process : snapshot) {
                visited.add(process);
                if (!process.isFinished()) {
                    // TODO: to avoid exceptions at the end of the simulation,
                    // these are being caught here. Maybe something can be fixed
                    // in the simulation so that the exception does not occur here.
                    try {
                        process.activate();
                    } catch (IllegalStateException e) {

                    }
                }
            }
        }

        // assert that all threads have been terminated.
        assert currentResourceTable.size() == 0;
    }
}
