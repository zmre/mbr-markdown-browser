---
author: PJW
created: 2026-10-07
type: flashcard
tags: [radio, study, learn]
---
# Ham Radio Technician Study Guide

I've copied these down from <https://hamstudy.org/>.


**T1-B03: Which frequency is in the 6-meter amateur band?**\
\
A. 49.00 MHz\
B. 52.525 MHz\
C. 28.50 MHz\
D. 222.15 MHz
: B. 52.525 MHz
: Convert wavelength to frequency using f (MHz) = 300 / λ (meters). For a 6-meter wavelength:\
\
$f (MHz) = 300 / 6 ≈ 50 MHz$\
\
The amateur 6-meter band runs from 50.0 MHz to 54.0 MHz in the U.S., so 52.525 MHz falls inside that allocation at 5.71 meters. The other listed frequencies are outside the 6-meter band (for example, 49.00 MHz is just below the 6-meter allocation, frequency-wise).

**T1-C04: What may happen if the FCC is unable to reach you by email?**\
\
A. Fine and suspension of operator license\
B. Revocation of the station license or suspension of the operator license\
C. Revocation of access to the license record in the FCC system\
D. Nothing; there is no such requirement
: **B.** Revocation of the station license or suspension of the operator license
: The FCC requires an email address to be on file as the primary contact method for every amateur station and operator license issued. The FCC will no longer mail physical correspondence.
: **§ 97.23 Mailing and email addresses.**
*Each license grant must show the grantee's correct name, mailing address, and email address. The email address must be an address where the grantee can receive electronic correspondence. Revocation of the station license or suspension of the operator license may result when correspondence from the FCC is returned as undeliverable because the grantee failed to provide the correct email address.*

**T2-A11: What term describes an amateur station that is transmitting and receiving on the same frequency?**\
\
A. Full duplex\
B. Diplex\
C. Simplex\
D. Multiplex
: **C.** Simplex
: `Simplex` communication refers to a configuration in which transmitting and receiving occur on the same frequency. Because both directions share one frequency, the operator can either send or receive at any one time but cannot do both simultaneously. In amateur radio, simplex means a single frequency is used for both transmitting and receiving, making it the simplest form of two-way communication.
: Memory aids:
  * "Simplex = Simple" (same frequency, one at a time)
  * Think "single" frequency for both transmit and receive
: ___Review History___
  * 2026-10-15 16:47 - Again
  * 2026-10-15 16:48 - Hard


## Subelement T2B: Operating Procedures

VHF/UHF operating practices: FM repeater, simplex, reverse splits; Access tones: CTCSS, DTMF; DMR operation; Resolving operating problems; Q signals

## Subelement T3A: Radio Wave Propagation

  Radio wave characteristics: how a radio signal travels, fading, multipath, polarization, wavelength vs absorption; Antenna orientation

  **T3-A01: Why do VHF signal strengths sometimes vary greatly when the antenna is moved only a few feet?**\
  \
  A. The signal path encounters different concentrations of water vapor\
  B. VHF ionospheric propagation is very sensitive to path length\
  C. Multipath propagation cancels or reinforces signals\
  D. The Doppler effect causes slight frequency shifts which result in changes in signal strength
: **C.** Multipath propagation cancels or reinforces signals
: **Multipath propagation**
: VHF signals often reach a receiving antenna by more than one route — for example a direct path plus reflections from the ground, buildings, water, or layers in the atmosphere. Each path has a slightly different length so the same radio wave can arrive with different phase. When those components add at the receiver they do so vectorially; they can reinforce each other (constructive interference) or cancel each other (destructive interference). A small physical movement of the antenna changes the relative path lengths by a fraction of a wavelength, which can flip the interference from constructive to destructive or vice versa. Because VHF wavelengths are on the order of a meter to a few meters, moving an antenna only a few feet is enough to produce large changes in received signal strength.

  **T3-A03: What antenna polarization is normally used for long-distance CW and SSB contacts on the VHF and UHF bands?**\
  \
  A. Right-hand circular\
  B. Left-hand circular\
  C. Horizontal\
  D. Vertical
: **C.** Horizontal
: Long-distance VHF and UHF contacts (DX) are usually made using horizontal polarization. Many long‑range agreements use antenna setups oriented horizontally (for example, horizontally mounted Yagi arrays) because horizontal polarization tends to work better for the common long‑distance propagation modes used on VHF/UHF, and matching the other station's polarization gives the strongest signal.
: Memory aids:
    * Think "horizon‑tal" for over‑the‑horizon (long‑distance) contacts.
    * Polarization must match between stations to get the best signal; use the same polarization the other stations are using.

  **T3-A05: When using a directional antenna, how might your station be able to communicate with a distant repeater if buildings or obstructions are blocking the direct line of sight path?**\
  \
  A. Change from vertical to horizontal polarization\
  B. Try to find a path that reflects signals to the repeater\
  C. Try the long path\
  D. Increase the antenna SWR
: **B.** Try to find a path that reflects signals to the repeater
: A directional (beam) antenna concentrates radiated energy into a narrower beam, so you can aim the main lobe at a specific direction. If direct line-of-sight to the repeater is blocked by buildings or other obstructions, you can aim the antenna at a nearby surface (roof, wall, water, metal structure) that will reflect the signal toward the repeater. The reflected path can carry enough energy to reach the repeater even though the straight-line path is obstructed. This is effectively using a deliberate multipath reflection to get around obstacles.
: Memory aids:
    * Think of it like playing pool: if you can't hit the target directly, aim at a cushion at the correct angle so the ball bounces to the target.
    * Surfaces that are large, flat, and conductive (metal roofs, large windows with thin metal or metallic oxide coatings, calm water) make good reflectors for VHF/UHF signals.

  **T3-A06: What is the meaning of the term "picket fencing"?**\
  \
  A. Alternating transmissions during a net operation\
  B. Rapid flutter on mobile signals due to multipath propagation\
  C. A type of ground system used with vertical antennas\
  D. Interference from cable TV in the form of carriers at fixed intervals across the band
: **B.** Rapid flutter on mobile signals due to multipath propagation
: "Picket fencing" describes rapid flutter or fading of a received signal as a mobile station moves. The effect is caused by multipath propagation: radio waves reach the receiver by multiple paths (direct and reflected), and as the relative phases of those paths change with movement they alternately add and cancel, producing quick rises and drops in signal strength.
: Memory aids:
    * Imagine watching a light through a picket fence: it appears and disappears as the gaps and slats pass — the same flicker happens to a signal experiencing multipath.
    * Often heard as a rapid flutter while driving with a handheld or mobile radio.

  **T3-A07: What weather condition might decrease range at microwave frequencies?**\
  \
  A. High winds\
  B. Low barometric pressure\
  C. Precipitation\
  D. Colder temperatures
: **C.** Precipitation
: Higher radio frequencies are more strongly absorbed and scattered by water and hydrometeors (raindrops, snowflakes, fog). At microwave frequencies, precipitation causes significant attenuation of the signal by converting RF energy into heat and by scattering the waves, which reduces range.
: Other weather factors listed have little direct effect on microwave propagation (wind might move or misalign antennas but doesn't absorb the signal; barometric pressure and modest temperature changes do not cause the same absorption/attenuation as precipitation).
: Memory aids:
  * Think of a household microwave oven: it uses a frequency that is readily absorbed by water to heat food. That same tendency for microwaves to be absorbed by water explains why rain, snow, or heavy fog reduce microwave radio range.

**T3-B06: What is the formula for converting frequency to approximate wavelength in meters?**\
\
A. Wavelength in meters equals frequency in hertz multiplied by 300\
B. Wavelength in meters equals frequency in hertz divided by 300\
C. Wavelength in meters equals frequency in megahertz divided by 300\
D. Wavelength in meters equals 300 divided by frequency in megahertz
: **D.** Wavelength in meters equals 300 divided by frequency in megahertz
: **Wavelength in meters equals 300 divided by frequency in megahertz.**
: $$\begin{align} \lambda _\text{ (meters)} = \frac{300}{f_\text{ (MHz)}} \end{align}$$
: Knowing this will help you with quite a few of the problems in the Technician class question pool!
: For example, if you see the frequency $150\text{ MHz}$ and need to know what band it is in, divide the speed of light by the frequency. MHz cancels out, which leaves you with $\frac{300}{150\text{ MHz}} = 2\text{ meters}$!
: The $150\text{ MHz}$ frequency is exactly in the middle of the 2-meter band. If your number is not quite on (e.g. $\frac{300}{144\text{ MHz}} = 2.08\text{ meters}$) that's okay, because the bands have a little play both above and below the "wavelength" number.
: **More Detail**
*Ever wonder where the value $300$ comes from?*
: Here's the general form of the equation above:
: $$\lambda \times f = c$$
: The units are all basic SI units (International System of Units or Metric System) -- let's add them for clarity:
: $$\lambda\_\text{ (meters)} \times f\_\text{ (Hz)} = c\_\text{ (meters per second)}$$
: The speed of light $c$ is a constant, so we can plug that value in: $$\begin{align} \lambda\_\text{ (meters)} \times f\_\text{ (Hz)} &= 3.00 \times 10^8\text{ m/s} \end{align}$$
: There are $10^6\text{ Hz}$ in each $\text{MHz}$, so let's divide both sides by $10^6$ and simplify: $$\begin{align} \frac{\lambda\_\text{ (meters)} \times f\_\text{ (Hz)}}{10^6} &= \frac{3.00 \times 10^8\text{ m/s}}{10^6}\ \lambda\_\text{ (meters)} \times \frac{f\_\text{ (Hz)}}{10^6} &= 3.00 \times 10^2\text{ m/s}\ \lambda\_\text{ (meters)} \times f\_\text{ (MHz)} &= 300 \text{ (units combined)}\ \lambda\_\text{ (meters)} &= \frac{ 300 }{ f\_\text{ (MHz)}}\ \end{align}$$
: Memory aids:\
"NUMber" goes in the "NUMerator" (300 divided by)

## Subelement T3C: Radio Wave Propagation

  Propagation modes: sporadic E, meteor scatter, auroral propagation, tropospheric ducting; F region skip; Line of sight and radio horizon

  **T3-C01: Why are simplex UHF signals rarely heard beyond their radio horizon?**\
  \
  A. They are too weak to go very far\
  B. FCC regulations prohibit them from going more than 50 miles\
  C. UHF signals are usually not propagated by the ionosphere\
  D. UHF signals are absorbed by the ionospheric D region
: **C.** UHF signals are usually not propagated by the ionosphere
: Simplex (direct) UHF signals are rarely heard beyond the radio horizon because the ionosphere does not normally reflect them back to Earth. Long-distance HF and some VHF signals can travel far by bouncing off the ionosphere, but the much shorter wavelengths at UHF pass through the ionosphere into space rather than being refracted back down. That is also why UHF frequencies are useful for satellite communication. As a result, when you hear a UHF station directly (not via a repeater or satellite), the transmitter is usually within line-of-sight or only a little beyond the radio horizon due to local propagation enhancements.
: Memory aids:
    * Higher frequency = straighter path (UHF passes through the ionosphere)
    * UHF works well for satellites — because it goes through the ionosphere, not bouncing off it

  **T3-C02: What is a characteristic of HF communication compared with communications on VHF and higher frequencies?**\
  \
  A. HF antennas are generally smaller\
  B. HF accommodates wider bandwidth signals\
  C. Long-distance ionospheric propagation is far more common on HF\
  D. There is less atmospheric interference (static) on HF
: **C.** Long-distance ionospheric propagation is far more common on HF
: **HF (high frequency) signals commonly travel long distances by reflecting or refracting from the ionosphere.** The HF portion of the spectrum (roughly 3–30 MHz) can be bent back toward Earth by ionized layers in the upper atmosphere, producing skywave or "skip" propagation that can carry signals hundreds or thousands of miles. By contrast, VHF and higher frequencies are mostly line-of-sight and do not reliably use ionospheric hops for long-distance contacts.
: Because of the lower frequencies, HF antennas are generally larger, HF signals tend to support narrower bandwidths for a given antenna/system design, and atmospheric and man-made noise is often greater at HF — all of which help explain why the standout characteristic of HF compared with VHF+ is the prevalence of long-distance ionospheric propagation.
: Memory aids:
    * Think of VHF as very "sharp" waves that tend to pass through the atmosphere — line-of-sight.
    * Think of HF as "dull" waves that can bounce off the ionosphere — long-distance.
    * "Skip" and "skywave" are common terms used to describe ionospheric hops on HF.

## Subelement T4A: Amateur Radio Practices

  Station setup: connecting a microphone, RF power meter, a power source, a computer, digital equipment, an SWR meter; bonding; Mobile radio installation

  **T4-A01: Which of the following is an appropriate power supply rating for a typical 50-watt output mobile FM transceiver?**\
  \
  A. 24.0 volts at 4 amperes\
  B. 13.8 volts at 4 amperes\
  C. 24.0 volts at 12 amperes\
  D. 13.8 volts at 12 amperes
: **D.** 13.8 volts at 12 amperes
: Most mobile FM transceivers are designed to run from automotive electrical systems, so the appropriate supply voltage is 13.8 volts (the nominal float/charging voltage for a 12‑volt lead‑acid system). That rules out any 24‑volt rating.
: Use P = E × I to check the required current. A supply rated 13.8 V × 4 A provides 55.2 watts of DC power, which is not enough because the radio is not 100% efficient. Mobile FM transmitters are typically only about 30–40% efficient, so to get 50 W of RF output you need roughly 50 W ÷ 0.35 ≈ 143 W of DC input. A 13.8 V × 12 A supply provides 165.6 W of DC power, which allows for the transmitter's inefficiency and other losses and is an appropriate rating for a 50 W output mobile FM transceiver.
  *Note:* 13.8 V is used because it is the common float/charging voltage for 12 V lead‑acid batteries; an engine running with a charging system may show a slightly higher voltage (around 14.4–14.7 V) and a resting battery will be lower (around 12.6–12.8 V).

  **T4-A02: Which of the following should be considered when selecting an accessory SWR meter?**\
  \
  A. The frequency and power level at which the measurements will be made\
  B. The distance that the meter will be located from the antenna\
  C. The maximum SWR anticipated on the transmission line\
  D. The ability of the meter to compensate for a poor transmission line match to the antenna
: **A.** The frequency and power level at which the measurements will be made
: An SWR meter measures the standing wave ratio in the antenna system, which indicates how closely the impedance of the transmitter matches the impedance of the antenna system. By the Maximum Power Transfer theorem, maximum power is transferred when the impedances are matched, so you choose an SWR meter based on the conditions under which you'll be measuring impedance and power.
: The two things that affect an SWR meter's suitability are the frequency of operation and the power level. SWR meters have frequency ranges (for example 0–150 MHz) and power handling limits (for example 100 W); using a meter outside its frequency range can give incorrect readings, and exceeding its power rating can damage the meter. Other considerations named in the question either do not affect which meter to buy or are misconceptions: where the meter is located relative to the antenna doesn't change which meter you should select (it may affect readings but it affects all meters equally), and a meter does not "compensate" for a poor match in the feed line — it only measures forward and reflected power so you can evaluate the match.
: Memory aids / mnemonics:
    * Remember: choose an SWR meter for the band (frequency) you use and the maximum power you'll put through it.
    * Think "frequency and power" as the two selection keys — everything else is secondary or a misconception.

## Subelement T4B: Amateur Radio Practices

  Operating controls: frequency tuning, use of filters, squelch function, AGC, memory channels, noise blanker, microphone gain, receiver incremental tuning (RIT), bandwidth selection, scanning function; Digital transceiver configuration; DMR code plugs and talk groups

  **T4-B01: What is the effect of excessive microphone gain on SSB transmissions?**\
  \
  A. Frequency instability\
  B. Distorted transmitted audio\
  C. Increased SWR\
  D. Sideband inversion
: **B.** Distorted transmitted audio
: Many microphones and radios provide a gain control for the microphone input. If that gain is set too high the audio waveform can exceed the linear range of the microphone preamplifier or the transmitter audio circuitry and the peaks become clipped. Clipping and overdrive produce distorted audio and intermodulation products that make speech harder to understand and can cause splatter into adjacent frequencies. Excessive microphone gain therefore degrades the transmitted audio quality rather than causing frequency instability, a change in SWR, or inversion of the sideband.
: Memory aids:
  * If the audio sounds harsh, buzzy, or unreadable, reduce the microphone gain — too much gain = clipped/distorted audio.

**T5-A03: What is the term for the flow of electrons in an electric circuit?**\
\
A. Voltage\
B. Amperes per second\
C. Volts per second\
D. Current
: **D.** Current
: Electric current is the flow of electric charge; in most circuits this is the flow of electrons. That flow is called current and is measured in amperes (amps) abbreviated A. For example, 10A. In formulas, current is represented by the letter I (for French intensité).\
For example, E = I x R\
Voltage is the electrical "pressure" that pushes the charges, and resistance opposes the flow.
: Memory aids:
  * Think of water: the flow of water = current; pressure = voltage; pipe constriction = resistance.
  * Current is measured in amperes (amps).

## Subelement T5D: Electrical Principles

  Ohm's Law; Series and parallel circuits

  **T5-D01: What formula is used to calculate current in a circuit?**\
  \
  A. I = E x R\
  B. I = E / R\
  C. I = E² x R\
  D. I = E² / R
: **B.** I = E / R
: Ohm's Law relates voltage (E, electromotive force), current (I), and resistance (R) by the equation E = I × R. To calculate current, solve that equation for I, which gives I = E / R — current equals voltage divided by resistance. Units: E in volts, R in ohms, I in amperes. This means current is directly proportional to voltage and inversely proportional to resistance; there is no squaring of E in the basic Ohm's Law relationship.
: Memory aids:
    * Think of E as an Eagle, I as an Igloo, and R as a Rabbit (Gordon West mnemonic).
    * If E is on the left, I and R are on the right (E = I × R).
    * If E is on the right, the Eagle is above the others, so I = E / R or R = E / I depending on which is above the other.
    * "Eagle over Rabbit" reminds you that I = E / R (Eagle above Rabbit).

  **T5-D08: What is the current through a 100-ohm resistor connected across 200 volts?**\
  \
  A. 20,000 amperes\
  B. 0.5 amperes\
  C. 2 amperes\
  D. 100 amperes
: **C.** 2 amperes
: Ohm's Law relates voltage, current, and resistance:
: E = I × R
: Where:

  * E is voltage in volts (V)
  * I is current in amperes (A)
  * R is resistance in ohms (Ω)
: We are given voltage and resistance and need current, so rearrange the formula:
: I = E / R
: Substitute the values:
: I = 200 V / 100 Ω = 2 A
: Memory aids:
  * Ohm's Law: E = I × R
  * To find current: I = E / R
  * To find voltage: E = I × R
  * To find resistance: R = E / I
  * (Reference) See Ohm's law on Wikipedia for more detail

  **T5-D13: In which type of circuit is the current always the same through all components?**\
  \
  A. Series\
  B. Parallel\
  C. Resonant\
  D. Branch
: **A.** Series
: **Explanation**
: In a series circuit there is only one path for current to flow, so the same current passes through every component in that path. Current is the rate of charge flow; because the charges that pass through one component must also pass through the next (there's no alternate route), the current is identical in each element. Voltage, however, can differ across each component and will add up to the total source voltage.
: In the animation below, the amount of voltage is indicated by the darkness of the green, and the current is represented by the "walking ant" animation.
: 
: Memory aids:

  * Series: the letter "S" can remind you that current is the "S"ame through all components.
: -Single path: Charges flow along only one continuous path.
: -No branching: There are no junctions or split roads where current can divide.
: -Charge conservation: Electrons cannot build up or disappear in the wire, meaning the flow rate remains constant everywhere in the loop.
  * Parallel is like train tracks — paths run side by side.
  * Series is like a television series — one episode after another.

  **T5-D14: In which type of circuit is the voltage always the same across all components?**\
  \
  A. Series\
  B. Parallel\
  C. Resonant\
  D. Branch
: **B.** Parallel
: In a parallel circuit each component is connected across the same two nodes, so the potential difference (voltage) across every component is identical even though the currents through them may differ.
: In the animation below, the amount of **voltage** is indicated by the darkness of the **green**, and the current is represented by the "walking ant" animation.
: 
: There is full source voltage on one side and no voltage on the other, and because each component is connected across those same two points, the voltage is the same across all components in a parallel circuit.

**T6-A01: What electrical component opposes the flow of current in a DC circuit?**\
\
A. Inductor\
B. Resistor\
C. Inverter\
D. Transformer
: **B.** Resistor
: A resistor is the component that opposes the flow of current in a DC circuit. It does this by converting electrical energy into heat, and the relationship between voltage, current, and resistance is given by Ohm's law (V = I·R). For a fixed voltage, a larger resistance results in a smaller current.
: An inductor does not oppose steady DC current; it resists changes in current (for example, in AC or during transients) but once DC is steady an ideal inductor behaves like a short circuit.
: 
: <https://en.wikipedia.org/wiki/Resistor>
: Memory aids:
  * Remember: inductors oppose changes in current, not steady DC

**T6-A05: What type of electrical component consists of conductive surfaces separated by an insulator?**\
\
A. Resistor\
B. Potentiometer\
C. Oscillator\
D. Capacitor
: **D.** Capacitor
: A capacitor is an electrical component that stores energy in an electric field. It consists of at least two conductive surfaces (plates) separated by an insulator (called a dielectric). When voltage is applied to the capacitor, charge accumulates on the plates, creating an electric field between them and storing energy.
: The schematic symbol for a capacitor resembles its physical construction: two parallel lines (plates) separated by a gap.
: When first connected to a voltage source, a capacitor will draw current as it charges; as the voltage across the capacitor approaches the source voltage, the charging current falls off and the capacitor no longer allows steady current to pass into it (it blocks DC once fully charged). If the voltage source is removed, the capacitor will discharge, supplying current until the stored energy in the electric field is depleted. Because a capacitor opposes rapid changes in voltage across its terminals, it is often described as resisting changes in voltage.
: Note: the question wording is broad, since other components (like resistors or potentiometers) are also made of conductive surfaces separated by insulating material in a mechanical sense, but the defining electrical component that specifically consists of conductive surfaces separated by an insulator and stores energy in an electric field is the capacitor.
: 

**T6-A06: What type of electrical component stores energy in a magnetic field?**\
\
A. Resistor\
B. Capacitor\
C. Inductor\
D. Diode
: **C.** Inductor
: There are two similar concepts often asked about: one component stores energy in an electric field, the other stores energy in a magnetic field. The component that stores energy in a magnetic field is an inductor. An inductor is typically a coil of wire.
: When current flows through the coil, a magnetic field is created around it. If the coil has a ferromagnetic (ferrite or iron) core, the core becomes magnetized and the magnetic effect is stronger — this is the same principle used to make electromagnets (for example, wrapping insulated wire around a nail to magnetize it when current flows).
: An inductor resists changes in current. When current is first applied, it opposes the change because the magnetic field is building. Once the field is established, the coil behaves more like a wire. If the current is suddenly interrupted, the collapsing magnetic field induces a reverse voltage (back EMF) that tries to prevent the current flowing, which can produce voltage spikes or sparks. This behavior is a direct consequence of the energy stored in the magnetic field of the inductor.
: Memory aids:
  * Think of the word "MICE": (M)agnetic field — (I)nductor; (C)apacitor — (E)lectric field.

**T6-A07: What electrical component is typically constructed as a coil of wire?**\
\
A. Transistor\
B. Capacitor\
C. Diode\
D. Inductor
: **D.** Inductor
: An inductor generally consists of a coil of wire.
:  The schematic symbol resembles the coil used in its construction.
: The properties of an inductor depend on whether the coil surrounds a magnetic core (such as ferrite). A magnetic core increases the magnetic field produced by a given current, which increases the inductance. Wrapping insulated wire around an iron nail and applying current is the classic way to make an electromagnet — the same basic idea as an inductor or transformer.
: When current through an inductor changes, the inductor resists that change by storing energy in its magnetic field. When current is turned on, the inductor initially opposes the rise in current; once the magnetic field is established, current can flow more easily. When the current is interrupted, the collapsing magnetic field attempts to keep the current flowing by inducing a voltage (back EMF), which can produce a large voltage spike as the energy is released.
: ___Review History___
  * 2026-10-15 16:47 - Hard
  * 2026-10-15 16:48 - Good

**T6-A10: Which of the following battery chemistries is rechargeable?**\
\
A. Nickel-metal hydride\
B. Lithium-ion\
C. Lead-acid\
D. All these choices are correct
: **D.** All these choices are correct
: All three named battery chemistries—nickel‑metal hydride (NiMH), lithium‑ion (Li-ion), and lead‑acid—are rechargeable. They are examples of secondary cells, meaning their electrochemical reactions can be reversed by applying an external current. By contrast, zinc‑carbon cells are primary batteries and are not intended to be recharged.
: Memory aids:
  * "Can't Zap a Carbon Zinc" (to remember zinc‑carbon batteries are not rechargeable)

**T6-A11: Which of the following battery chemistries is not rechargeable?**\
\
A. Nickel-cadmium\
B. Carbon-zinc\
C. Lead-acid\
D. Lithium-ion
: **B.** Carbon-zinc
: Carbon–zinc batteries are primary cells whose chemical reactions are not readily reversible, so they cannot be effectively recharged. In contrast, nickel–cadmium, lead–acid, and lithium‑ion batteries are designed with reversible electrochemical reactions and are rechargeable (secondary) cell types.
: Memory aids:
  * "Can't Zap a Carbon‑Zinc" — C Z = Carbon‑zinc (can't be recharged)
  * Think "primary = disposable" for carbon‑zinc cells and "secondary = rechargeable" for NiCd, lead‑acid, and Li‑ion.

**T6-D09: What is the name of a device that combines several semiconductors and other components into one package?**\
\
A. Transducer\
B. Multi-pole relay\
C. Integrated circuit\
D. Transformer
: **C.** Integrated circuit
: An integrated circuit (IC), commonly called a microchip, is a single packaged device that contains multiple semiconductor devices (such as transistors and diodes) and other components (like resistors and capacitors) interconnected to perform one or more electronic functions. The components are fabricated together on a small piece of semiconductor material and then enclosed in a package so the whole assembly can be used as a single component. Examples range from simple logic chips to complex CPUs and memory devices; nearly every modern circuit board includes ICs.\
: In schematic diagrams, IC chips are labeled U# (U1, U2, etc.).

**T7-A07: What is the function of a transceiver's PTT input?**\
\
A. Input for a key used to send CW\
B. Switches transceiver from receive to transmit when grounded\
C. Provides a transmit tuning tone when grounded\
D. Input for a preamplifier tuning tone
: **B.** Switches transceiver from receive to transmit when grounded
: The PTT input (Push-to-Talk) is an external control that switches the transceiver from receive to transmit when activated and switches the antenna from the receiver to the transmitter. When the PTT input is not active, the transceiver remains in receive mode. Activating the PTT input causes the transceiver to stop receiving and begin transmitting. Releasing it returns the transceiver back to receive mode.\
For digital modes (FT8, etc.), PTT is often implemented via a computer interface (like CAT control) simulating the physical button press.
: Memory aids:
  * PTT = Push To Talk
  * Activation is often done by grounding the PTT input (closing a switch), such as the microphone's transmit switch.

**T7-A08: Which of the following describes combining speech with an RF carrier signal?**\
\
A. Impedance matching\
B. Oscillation\
C. Modulation\
D. Low-pass filtering
: **C.** Modulation
: A modulator is the circuit that "modulates" or encodes the RF carrier frequency with the speech signal in a transmitter. The keyword "combines" indicates that the voice and carrier are put together, which should help you envision a circuit that modifies the carrier (for example by varying its amplitude, frequency, or phase) so it carries the audio information.
: Memory aid: 
  * Think of "modulation" as putting information onto a carrier wave so it can be sent over the air.
  * Alternative memory aid: "Modulation" sounds similar to "modify". The modular circuit "modifies" the carrier.

**T8-C05: What is a grid locator?**\
\
A. A letter-number designator assigned to a geographic location\
B. A letter-number designator assigned to an azimuth and elevation\
C. An instrument for locating faults in power amplifiers\
D. An instrument for radio direction finding
: **A.** A letter-number designator assigned to a geographic location
: A grid locator (often called a Maidenhead grid square or grid square) is a short letter–number code that represents a specific geographic location by encoding latitude and longitude. Amateur radio operators use these grid locators to report where a station is located for contests, awards, propagation studies, and logging. It is a location designator, not a physical instrument or a measurement of azimuth/elevation.\
[AMSAT: What's my Grid Square?](https://www.amsat.org/whats-my-grid-square/)
: Memory aids:
  * Think of a map's grid: a grid locator tells you where on the map (geographic location) a station is located.
  * Locator = Location

**T8-D03: What kind of data can be transmitted by APRS?**\
\
A. GPS position data\
B. Text messages\
C. Weather data\
D. All these choices are correct
: **D.** All these choices are correct
: APRS (Automatic Packet Reporting System) is a two‑way tactical real‑time digital communications system for sharing information about what's happening in the local area. Because it's a general-purpose packet system for short, periodic updates and telemetry, it can carry GPS position reports (to show station location), short text messages between stations, and telemetry such as weather‑station data. See <http://aprs.org/> for more details.\
See <https://aprs.fi> for APRS maps.
