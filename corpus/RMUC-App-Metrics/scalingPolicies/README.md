# README

## Defined Elastic Infrastructures (EI) and Service Groups (SG)
In `scalingPolicies/models2023EI.spd` :
* EI *RMUC Environment* : entire Resource Environment of the RMUC
  * with a `Group Size Constraint` of at most 5 and at least 1 container.
  * with a `Thrashing Constraint` of at least 1.0 second of no thrashing. 

In `scalingPolicies/models2023SG.spd` :
* SG *DataProvider* : targets the assembly `DataProvider`
  * with a `Group Size Constraint` of at most 4 and at least 1 instance.
* SG *DeviceCommunication* : targets the assembly `DeviceCommunication`
  * with a `Group Size Constraint` of at most 5 and at least 1 instance.

For both `.spd` files exists a corresponding semantic mapping (`.semanticspd`). 
The mappings are identical except for the referenced `.spd` file.
Please combine the as follows:
* `scalingPolicies/models2023EI.spd` and  `scalingPolicies/semanticModels/models2023EI.semanticspd`
* `scalingPolicies/models2023SG.spd` and  `scalingPolicies/semanticModels/models2023SG.semanticspd`


## Elastic Infrastructure (EI) Target Groups

The policies for targeting an EI are defined in `scalingPolicies/models2023EI.spd`.

### UC 1 : Scaling Out the ElasticInfrastructure at a certain point in time to a absolute value.
* Activate Scaling Policy *UC1_UC2_PointInTimeScaleOut*.

### UC 2 : Scaling Out and In the ElasticInfrastructure at two different points in time.
* Activate Scaling Policy *UC1_UC2_PointInTimeScaleOut* to scale out and activate Scaling Policy *UC2_PointInTimeScaleIn* for a subsequent scale in.

### UC 3 : Scaling Out the ElasticInfrastructure based on CPU Utilization to an absolute value.
* Activate Scaling Policy *UC3_AbsoluteCPUScaleOut*.

**!!** : not yet tested, as CPU Trigger is still incomplete.  

### UC 4 : Scaling Out the ElasticInfrastructure based on CPU Utilization relatively.
* Activate Scaling Policy *UC4_RelativeCPUScaleOut*. 

**!!** : not yet tested, as CPU Trigger is still incomplete.  

### UC 5 : Scaling Out the ElasticInfrastructure based on SLOs (ResponseTime).
* Activate Scaling Policy *UC5_SLOBasedScaleOut*.

**!!** : not yet tested, as Operation Response Time Measurements are still missing (c.f. respective PR).  

### UC 6 : Group size constraints.
* Activate Scaling Policy *UC6_GroupsizeConstraint*.
* The Policy attempts an absolute adjustment to 10, but the constraint defined on the EI restricts the adjustment to 5 resource containers. 

### UC 7 : Cool down constraints.
**!!** cannot be modeled, because the cool down is somehow not available in the tree editor.

### UC 7-1 : Interval constraints.
* Activate Scaling Policy *UC7-1_IntervalConstraint*. 
* The Intervall has a duration of 3 seconds and an offset of 2 second, i.e.
  - \[0.0, 2.0\) : scaling is blocked
  - \[2.0, 5.0\) : scaling is allowed
  - \[5.0, 7.0\) : scaling is blocked
  - \[7.0, 10.0\) : scaling is allowed
  - ... 
* The policy has a composed point in time trigger (`OR`), for the points in time $0.5$ (blocked), $2.5$ (allowed), $5.0$ (blocked) and $7.0$ (allowed).

* *Beware* : duration between subsequent point in time triggers must be greater than $1$ second, to avoid clashed with the EI's thrashing constraint.

**!** currently not working, but i'm pretty sure it will, once we get the constraints x aggregation PR :) (because it worked when we tested last week)

**!** are upper/lower bound in- or exclusive? 

### UC 8 : Thrashing constraints
* Activate Scaling Policy *UC8_ThrashingConstraint*.

- The policy is defined for *Constraint RMUC Environment*, which enforces no thrashing for $1.0$ second. 
- The policy has a composed point in time trigger (`OR`), for the points in time $0.1$ (allowed), $1.0$ (blocked), $2.0$ (allowed).



## Service Group (SG) Target Groups

The policies for targeting SGs are defined in `scalingPolicies/models2023SG.spd`.

### UC 9	: Scaling Out one Service at a point in time to an absolute value
* Activate Scaling Policy *UC9_PointInTimeScaleOut*.

### UC 10 : Target Groups Constraints for two target groups
* Activate Scaling Policies *UC10_ConstraintOnDataProvider* and *UC10_ConstraintOnDeviceCommunication*.
* Both trigger at the same point in time and scale out to an absolute value greater then their respective SG group constraints. 

### UC 11 : Scaling Out one Service step based on CPU Utilization
* Activate Scaling Policy *UC11_StepBasedCPUScaleOut*.

**!** wont work, as CPU trigger is still lacking. 

### UC 12 : add one scale out policy after observing a bottleneck shift
* Activate Scaling Policy *UC12_BottleneckShift* to witness a bottleneck shift.
* Activate Scaling Policies *UC12_BottleneckShift* and *UC12_BottleneckMitigation* to witness the resolution of the bottleneck shift.

**TODO** Details not yet modelled, as it's unclear how to create a bottle neck shift in the RMUC o___O 
