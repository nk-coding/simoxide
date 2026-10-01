package de.uka.ipd.sdq.simucomframework.core.resources;

import java.util.ArrayDeque;
import java.util.Queue;

import org.palladiosimulator.pcm.core.composition.AssemblyContext;
import org.palladiosimulator.pcm.repository.PassiveResource;

import de.uka.ipd.sdq.scheduler.IPassiveResource;
import de.uka.ipd.sdq.scheduler.ISchedulableProcess;
import de.uka.ipd.sdq.scheduler.LoggingWrapper;
import de.uka.ipd.sdq.scheduler.SchedulerModel;
import de.uka.ipd.sdq.scheduler.processes.IWaitingProcess;
import de.uka.ipd.sdq.scheduler.processes.SimpleWaitingProcess;
import de.uka.ipd.sdq.scheduler.resources.AbstractSimResource;
import de.uka.ipd.sdq.scheduler.resources.passive.PassiveResourceObservee;
import de.uka.ipd.sdq.scheduler.sensors.IPassiveResourceSensor;
import de.uka.ipd.sdq.simucomframework.core.exceptions.FailureException;
import de.uka.ipd.sdq.simucomframework.core.model.SimuComModel;

/**
 * Simulates a simple passive resource.
 *
 * Note: This class intentionally does not extend {@link SimAbstractPassiveResource}, because that
 * abstract class is intended for passive resources that are accessed by EXACT schedulers (e.g.,
 * specific Windows, Linux Scheduler).
 *
 * TODO: comment
 *
 * TODO Initialize based on given model elements [Lehrig]
 *
 * @author Fabro, Sebastian Lehrig
 *
 * @param <M>
 *            the type of the simulation model.
 */
public class SimSimpleFairPassiveResource extends AbstractSimResource implements IPassiveResource {

    protected Queue<IWaitingProcess> waitingQueue;
    private final SchedulerModel myModel;
    private long available;
    private final String passiveResourceID;
    private final boolean simulateFailures;

    // provides observer functionality to this resource
    private final PassiveResourceObservee observee;
    private final PassiveResource resource;
    private final AssemblyContext assemblyContext;

    public SimSimpleFairPassiveResource(final PassiveResource resource, final AssemblyContext assemblyContext,
            final SchedulerModel model, final Long capacity) {
        super(model, capacity, resource.getEntityName(), resource.getId() + ":" + assemblyContext.getId());
        this.resource = resource;
        this.assemblyContext = assemblyContext;

        this.waitingQueue = new ArrayDeque<>();
        this.myModel = model;
        this.passiveResourceID = resource.getId();
        this.observee = new PassiveResourceObservee();
        this.available = capacity;

        /*
         * The following workaround can be removed once failure simulation has been factored out of
         * this class or has been made independent of SimuComModel.
         * 
         * Actually, neither this class nor the failure-enhanced class should prescribe the concrete
         * simulation model because this excludes simulators other than SimuCom from reusing this
         * class. Instead, this class should require an abstract (!) simulation model like
         * ISimulationModel.
         */
        if (myModel instanceof SimuComModel) {
            this.simulateFailures = ((SimuComModel) model).getConfiguration()
                .getSimulateFailures();
        } else {
            this.simulateFailures = false;
        }
    }

    private boolean canProceed(final ISchedulableProcess process, final long num) {
        return (waitingQueue.isEmpty() || waitingQueue.peek()
            .getProcess()
            .equals(process)) && num <= available;
    }

    @Override
    public PassiveResource getResource() {
        return this.resource;
    }

    @Override
    public AssemblyContext getAssemblyContext() {
        return this.assemblyContext;
    }

    @Override
    public Queue<IWaitingProcess> getWaitingProcesses() {
        return waitingQueue;
    }

    private void grantAccess(final ISchedulableProcess process, final long num) {
        LoggingWrapper.log("Process " + process + " acquires " + num + " of " + this);
        this.available -= num;
        if (refsim.trace.Trace.ON) { // REFSIM trace
            refsim.trace.Trace.event("grant", process, "res", passiveResourceID, "ac", assemblyContext.getId(), "n", num,
                    "avail", available);
        }
        observee.fireAquire(process, num);
        assert this.available >= 0 : "More resource than available have been acquired!";
    }

    @Override
    public boolean acquire(final ISchedulableProcess schedulableProcess, final long num, final boolean timeout,
            final double timeoutValue) {

        // AM: Copied from AbstractActiveResource: If simulation is stopped,
        // allow all processes to finish
        if (!myModel.getSimulationControl()
            .isRunning()) {
            // Do nothing, but allows calling process to complete
            return true;
        }
        // TODO
        // Do we need some logic here to check if the simulation has stopped?
        // In this case, this method should not block, but return in order to
        // allow processes to complete
        observee.fireRequest(schedulableProcess, num);
        if (refsim.trace.Trace.ON) { // REFSIM trace
            refsim.trace.Trace.event("acquire", schedulableProcess, "res", passiveResourceID, "ac", assemblyContext.getId(), "n",
                    num, "avail", available, "queue", waitingQueue.size());
        }
        if (canProceed(schedulableProcess, num)) {
            grantAccess(schedulableProcess, num);
            return true;
        } else {
            LoggingWrapper.log("Process " + schedulableProcess + " is waiting for " + num + " of " + this);
            final SimpleWaitingProcess process = new SimpleWaitingProcess(myModel, schedulableProcess, num);
            processTimeout(timeout, timeoutValue, process);
            waitingQueue.add(process);
            schedulableProcess.passivate();
            return false;
        }
    }

    /**
     * Schedules a timeout event if a timeout is specified and failures are simulated.
     *
     * @param timeout
     *            indicates if the acquire request is associated with a timeout
     * @param timeoutValue
     *            the timeout value
     * @param process
     *            the waiting process
     */
    private void processTimeout(final boolean timeout, final double timeoutValue, final SimpleWaitingProcess process) {
        if (!simulateFailures || !timeout) {
            return;
        }
        // this cast is safe because simulateFailure is true if and only if myModel is a
        // SimuComModel
        SimuComModel simuComModel = (SimuComModel) myModel;
        if (timeoutValue == 0.0) {
            FailureException.raise(simuComModel, simuComModel.getFailureStatistics()
                .getResourceTimeoutFailureType(this.assemblyContext.getId(), this.passiveResourceID));
        }
        if (timeoutValue > 0.0) {
            final PassiveResourceTimeoutEvent event = new PassiveResourceTimeoutEvent(simuComModel, myModel, this,
                    process);
            event.schedule(process, timeoutValue);
        }
    }

    /**
     * Retrieves the passive resource ID.
     *
     * @return the passive resource ID
     */
    protected String getPassiveResourceID() {
        return passiveResourceID;
    }

    @Override
    public void release(final ISchedulableProcess schedulableProcess, final long num) {

        // AM: Copied from AbstractActiveResource: If simulation is stopped,
        // allow all processes to finish
        if (!myModel.getSimulationControl()
            .isRunning()) {
            // Do nothing, but allows calling process to complete
            return;
        }

        LoggingWrapper.log("Process " + schedulableProcess + " releases " + num + " of " + this);
        this.available += num;
        if (refsim.trace.Trace.ON) { // REFSIM trace
            refsim.trace.Trace.event("release", schedulableProcess, "res", passiveResourceID, "ac", assemblyContext.getId(), "n",
                    num, "avail", available);
        }
        observee.fireRelease(schedulableProcess, num);
        notifyWaitingProcesses();
    }

    private void notifyWaitingProcesses() {
        SimpleWaitingProcess waitingProcess = (SimpleWaitingProcess) waitingQueue.peek();
        while (waitingProcess != null && canProceed(waitingProcess.getProcess(), waitingProcess.getNumRequested())) {
            grantAccess(waitingProcess.getProcess(), waitingProcess.getNumRequested());
            waitingQueue.remove();
            waitingProcess.getProcess()
                .activate();
            waitingProcess = (SimpleWaitingProcess) waitingQueue.peek();
        }
    }

    @Override
    public void addObserver(final IPassiveResourceSensor observer) {
        observee.addObserver(observer);
    }

    @Override
    public void removeObserver(final IPassiveResourceSensor observer) {
        observee.removeObserver(observer);
    }

    @Override
    public long getAvailable() {
        return available;
    }

    /**
     * Determines if a given process is currently waiting to acquire this resource.
     *
     * @param process
     *            the process
     * @return TRUE if the process is waiting to acquire the resource; FALSE otherwise
     */
    public boolean isWaiting(final SimpleWaitingProcess process) {
        return waitingQueue.contains(process);
    }

    /**
     * Removes a waiting process from the queue.
     *
     * @param process
     *            the process to remove
     */
    public void remove(final SimpleWaitingProcess process) {
        waitingQueue.remove(process);
    }
}