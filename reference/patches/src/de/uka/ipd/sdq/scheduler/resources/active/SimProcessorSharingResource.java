package de.uka.ipd.sdq.scheduler.resources.active;

import java.util.ArrayList;
import java.util.Hashtable;
import java.util.List;
import java.util.Map.Entry;

import org.apache.log4j.Level;
import org.apache.log4j.Logger;

import de.uka.ipd.sdq.probfunction.math.util.MathTools;
import de.uka.ipd.sdq.scheduler.ISchedulableProcess;
import de.uka.ipd.sdq.scheduler.LoggingWrapper;
import de.uka.ipd.sdq.scheduler.SchedulerModel;
import de.uka.ipd.sdq.scheduler.entities.SchedulerEntity;
import de.uka.ipd.sdq.simulation.abstractsimengine.AbstractSimEventDelegator;

public class SimProcessorSharingResource extends AbstractActiveResource {

    private static final Logger LOGGER = Logger.getLogger(SimProcessorSharingResource.class);

    /**
     * The minimum amount of time used for scheduling an event
     */
    private final static double JIFFY = 1e-9;

    private final ProcessingFinishedEvent processingFinished;
    // REFSIM (same patch as reference/oracles/sched): Hashtable -> LinkedHashMap so that ties in
    // scheduleNextEvent() (first process with minimal remaining demand in iteration order) are broken by
    // insertion order instead of identity-hash order (which differs between runs).
    private final java.util.LinkedHashMap<ISchedulableProcess, Double> running_processes = new java.util.LinkedHashMap<ISchedulableProcess, Double>();
    /** Keeps track of the current number of processes assigned to each core. */
    private final List<Integer> numberProcessesOnCore;

    private double last_time;

    private class ProcessingFinishedEvent extends AbstractSimEventDelegator<ISchedulableProcess> {

        public ProcessingFinishedEvent(final SchedulerModel model) {
            super(model, ProcessingFinishedEvent.class.getName());
        }

        @Override
        public void eventRoutine(final ISchedulableProcess process) {
            final ISchedulableProcess last = process;
            toNow();
            running_processes.remove(last);

            reportCoreUsage();

            fireDemandCompleted(last);
            LoggingWrapper.log(last + " finished.");
            scheduleNextEvent();
            last.activate();
        }

    }


    public SimProcessorSharingResource(final SchedulerModel model, final String name, final String id, final long capacity, IResourceTableManager resourceTableManager) {
        super(model, capacity, name, id, resourceTableManager);
        this.processingFinished = new ProcessingFinishedEvent(model);
        this.numberProcessesOnCore = new ArrayList<Integer>((int) getCapacity());
        for (int i = 0; i < getCapacity(); i++) {
        	numberProcessesOnCore.add(0);
        }
    }

    public void scheduleNextEvent() {
        ISchedulableProcess shortest = null;
        for (final ISchedulableProcess process : running_processes.keySet()) {
            if (shortest == null || running_processes.get(shortest) > running_processes.get(process)) {
                shortest = process;
            }
        }
        processingFinished.removeEvent();
        if (shortest != null) {
            double remainingTime = running_processes.get(shortest) * getProcessingDelayFactorPerProcess();

            // avoid trouble caused by rounding issues
            remainingTime = remainingTime < JIFFY ? 0.0 : remainingTime;

            assert remainingTime >= 0 : "Remaining time (" + remainingTime + ")small than zero!";

            processingFinished.schedule(shortest, remainingTime);
        }
    }

    private void toNow() {
        final double now = getModel().getSimulationControl().getCurrentSimulationTime();
        double passed_time = now - last_time;
        double processedDemandperThread = passed_time / getProcessingDelayFactorPerProcess();
        if (MathTools.less(0, passed_time)) {
            for (final Entry<ISchedulableProcess, Double> e : running_processes.entrySet()) {
                final double rem = e.getValue() - processedDemandperThread;
                e.setValue(rem);
            }
        }
        last_time = now;
    }

    private double getProcessingDelayFactorPerProcess() {
        final double speed = (double) running_processes.size() / (double) getCapacity();
        return speed < 1.0 ? 1.0 : speed;
    }

    @Override
    public void start() {
    }

    @Override
    protected void dequeue(final ISchedulableProcess process) {
    }

    @Override
    protected void doProcessing(final ISchedulableProcess process, final int resourceServiceID, double demand) {
        toNow();
        LoggingWrapper.log("PS: " + process + " demands " + demand);
        if (demand < JIFFY) {
            demand = JIFFY;
            LoggingWrapper.log("PS: " + process + " demand was increased to match JIFFY " + demand);
        }

        running_processes.put(process, demand);

        reportCoreUsage();

        scheduleNextEvent();
        process.passivate();
    }

	/**Report core usage for each individual (virtual) core.
	 * The first core always has the most processes, which ensures backward-compatibility
	 * to the time when only one core was reported to be used if at least on process was running.
	 */
	private void reportCoreUsage() {
		if (running_processes.size() < getCapacity()) {
			for (int i = 0; i < getCapacity(); i++) {
				if (i < running_processes.size()) {
					// one process active on respective core
					assignProcessesAndFireStateChange(1,i);
				} else {
					// no process active on respective core
					assignProcessesAndFireStateChange(0,i);
				}
			}
		} else { // distribute across cores
			int minNumberProcessPerCore = (int) Math.floor((double) running_processes.size() / (double) getCapacity());
			int numberAdditionalProcesses = (int) (running_processes.size() - (minNumberProcessPerCore * getCapacity()));
			int numberProcessesAtCore;
 			for (int i = 0; i < getCapacity(); i++) {
				// distribute processes evenly across cores. The first
				// (getCapactity()-1) cores can have one additional process
				// beyond the minimal number of processes.
				if (numberAdditionalProcesses > 0) {
					numberProcessesAtCore = minNumberProcessPerCore + 1;
					numberAdditionalProcesses--;
				} else {
					numberProcessesAtCore = minNumberProcessPerCore;
				}
				assignProcessesAndFireStateChange(numberProcessesAtCore, i);
			}
		}
	}

    /**Assigns the provided number of processes to a core and fires a state change event if necessary.
     * @param targetNumberProcessesAtCore New number of active processes on the core. 
     * @param coreNumber Number of the core, starting with 0.
     */
    private void assignProcessesAndFireStateChange(int targetNumberProcessesAtCore, int coreNumber) {
    	if (!numberProcessesOnCore.get(coreNumber).equals(targetNumberProcessesAtCore)) {
    		numberProcessesOnCore.set(coreNumber, targetNumberProcessesAtCore);
    		fireStateChange(targetNumberProcessesAtCore, coreNumber);
    	}
	}

	@Override
    public double getRemainingDemand(final ISchedulableProcess process) {
        if (!running_processes.containsValue(process)) { // REFSIM: Hashtable.contains == containsValue (original bug kept)
            return 0.0;
        }
        toNow();
        return running_processes.get(process);
    }

    @Override
    public void updateDemand(final ISchedulableProcess process, final double demand) {
        boolean updated = false;
        for (final Entry<ISchedulableProcess, Double> e : running_processes.entrySet()) {
            if (e.getKey().equals(process)) {
                if (Double.isNaN(demand)) {
                    if (LOGGER.isEnabledFor(Level.INFO)) {
                        LOGGER.info("Specified demand " + demand + "is not a number.");
                    }
                }
                e.setValue(demand);
                updated = true;
                break;
            }
        }
        if (updated == false) {
            throw new RuntimeException("COULD NOT UPDATE PROCESS!");
        }
        scheduleNextEvent();
    }

    @Override
    protected void enqueue(final ISchedulableProcess process) {
    }

    @Override
    public void registerProcess(final ISchedulableProcess process) {
    }

    @Override
    public int getQueueLengthFor(final SchedulerEntity schedulerEntity, final int coreID) {
    	return numberProcessesOnCore.get(coreID);
    }

    @Override
    public void stop() {
    }

}
